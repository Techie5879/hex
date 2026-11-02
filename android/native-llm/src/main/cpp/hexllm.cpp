// JNI over llama.cpp for local dictation cleanup: greedy (temperature 0) as on
// desktop, cancellable and deadline-bounded per token. A run that doesn't reach
// end-of-generation throws, so a truncated rewrite can never replace the input.
#include <android/log.h>
#include <jni.h>
#include <llama.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <memory>
#include <string>
#include <vector>

#define LOG_TAG "HexLlm"
#define LOGE(...) __android_log_print(ANDROID_LOG_ERROR, LOG_TAG, __VA_ARGS__)

namespace {

struct Engine {
    llama_model *model = nullptr;
    llama_context *context = nullptr;
    int threads = 4;
    std::atomic<bool> cancelled{false};
    std::atomic<int64_t> deadline_ms{0};
};

int64_t now_ms() {
    using namespace std::chrono;
    return duration_cast<milliseconds>(steady_clock::now().time_since_epoch()).count();
}

bool should_abort(void *user) {
    auto *engine = static_cast<Engine *>(user);
    if (engine->cancelled.load()) return true;
    const int64_t deadline = engine->deadline_ms.load();
    return deadline > 0 && now_ms() >= deadline;
}

std::string to_string(JNIEnv *env, jstring value) {
    const char *chars = env->GetStringUTFChars(value, nullptr);
    std::string out(chars);
    env->ReleaseStringUTFChars(value, chars);
    return out;
}

void throw_engine(JNIEnv *env, const std::string &message) {
    jclass type = env->FindClass("anomaly/hex/android/nativellm/LlmException");
    if (type != nullptr) env->ThrowNew(type, message.c_str());
}

std::string stop_reason(Engine *engine) { return engine->cancelled.load() ? "cancelled" : "deadline"; }

// Valid UTF-8 for NewStringUTF: Java's modified UTF-8 can't take 4-byte sequences directly.
jstring new_string(JNIEnv *env, const std::string &text) {
    jbyteArray bytes = env->NewByteArray(static_cast<jsize>(text.size()));
    env->SetByteArrayRegion(bytes, 0, static_cast<jsize>(text.size()), reinterpret_cast<const jbyte *>(text.data()));
    jclass string_class = env->FindClass("java/lang/String");
    jmethodID init = env->GetMethodID(string_class, "<init>", "([BLjava/lang/String;)V");
    jstring charset = env->NewStringUTF("UTF-8");
    return static_cast<jstring>(env->NewObject(string_class, init, bytes, charset));
}

}  // namespace

extern "C" JNIEXPORT jlong JNICALL
Java_anomaly_hex_android_nativellm_NativeLlm_nativeOpen(JNIEnv *env, jclass, jstring path, jint threads, jint n_ctx) {
    static std::atomic<bool> once{false};
    if (!once.exchange(true)) llama_backend_init();
    llama_model_params model_params = llama_model_default_params();
    model_params.n_gpu_layers = 0;
    model_params.load_mode = LLAMA_LOAD_MODE_MMAP;
    const std::string file = to_string(env, path);
    llama_model *model = llama_model_load_from_file(file.c_str(), model_params);
    if (model == nullptr) {
        throw_engine(env, "Could not load the cleanup model");
        return 0;
    }
    llama_context_params context_params = llama_context_default_params();
    context_params.n_ctx = static_cast<uint32_t>(n_ctx);
    context_params.n_batch = 256;
    context_params.n_threads = threads;
    context_params.n_threads_batch = threads;
    llama_context *context = llama_init_from_model(model, context_params);
    if (context == nullptr) {
        llama_model_free(model);
        throw_engine(env, "Could not start the cleanup model");
        return 0;
    }
    auto engine = std::make_unique<Engine>();
    engine->model = model;
    engine->context = context;
    engine->threads = threads;
    llama_set_abort_callback(context, should_abort, engine.get());
    return reinterpret_cast<jlong>(engine.release());
}

extern "C" JNIEXPORT jstring JNICALL
Java_anomaly_hex_android_nativellm_NativeLlm_nativeComplete(JNIEnv *env, jclass, jlong handle, jstring prompt_text, jint max_tokens, jlong deadline_ms) {
    auto *engine = reinterpret_cast<Engine *>(handle);
    engine->cancelled = false;
    engine->deadline_ms = deadline_ms > 0 ? now_ms() + deadline_ms : 0;
    llama_memory_clear(llama_get_memory(engine->context), true);

    const std::string prompt = to_string(env, prompt_text);
    const llama_vocab *vocab = llama_model_get_vocab(engine->model);
    int count = -llama_tokenize(vocab, prompt.c_str(), static_cast<int32_t>(prompt.size()), nullptr, 0, true, true);
    if (count <= 0) {
        throw_engine(env, "Could not tokenize the prompt");
        return nullptr;
    }
    std::vector<llama_token> tokens(count);
    llama_tokenize(vocab, prompt.c_str(), static_cast<int32_t>(prompt.size()), tokens.data(), count, true, true);
    if (static_cast<uint32_t>(count + max_tokens) > llama_n_ctx(engine->context)) {
        throw_engine(env, "Transcript is too long for local cleanup");
        return nullptr;
    }

    auto sampler = std::unique_ptr<llama_sampler, decltype(&llama_sampler_free)>(
        llama_sampler_chain_init(llama_sampler_chain_default_params()), llama_sampler_free);
    llama_sampler_chain_add(sampler.get(), llama_sampler_init_greedy());

    // One decode takes at most n_batch tokens: a larger one aborts, so the prompt goes in pieces.
    const int batch_limit = static_cast<int>(llama_n_batch(engine->context));
    auto decode = [&](llama_token *start, int length) -> bool {
        if (should_abort(engine)) {
            throw_engine(env, stop_reason(engine));
            return false;
        }
        const int32_t status = llama_decode(engine->context, llama_batch_get_one(start, length));
        if (status != 0) {
            throw_engine(env, status == 2 ? stop_reason(engine) : "Cleanup model failed to decode");
            return false;
        }
        return true;
    };
    for (int offset = 0; offset < count; offset += batch_limit) {
        if (!decode(tokens.data() + offset, std::min(batch_limit, count - offset))) return nullptr;
    }

    std::string output;
    llama_token current = 0;
    for (int produced = 0; produced < max_tokens; produced++) {
        llama_token next = llama_sampler_sample(sampler.get(), engine->context, -1);
        if (llama_vocab_is_eog(vocab, next)) return new_string(env, output);
        char piece[256];
        const int length = llama_token_to_piece(vocab, next, piece, sizeof(piece), 0, false);
        if (length > 0) output.append(piece, static_cast<size_t>(length));
        llama_sampler_accept(sampler.get(), next);
        current = next;
        if (!decode(&current, 1)) return nullptr;
    }
    throw_engine(env, "Cleanup returned incomplete text");
    return nullptr;
}

extern "C" JNIEXPORT void JNICALL
Java_anomaly_hex_android_nativellm_NativeLlm_nativeCancel(JNIEnv *, jclass, jlong handle) {
    if (handle != 0) reinterpret_cast<Engine *>(handle)->cancelled = true;
}

extern "C" JNIEXPORT void JNICALL
Java_anomaly_hex_android_nativellm_NativeLlm_nativeClose(JNIEnv *, jclass, jlong handle) {
    if (handle == 0) return;
    auto *engine = reinterpret_cast<Engine *>(handle);
    llama_free(engine->context);
    llama_model_free(engine->model);
    delete engine;
}
