// JNI over transcribe.cpp: one session, one run at a time, cancellable between
// decode steps. Audio arrives as a path to raw PCM16 mono 16 kHz so a long
// dictation never crosses JNI or the Java heap.
#include <android/log.h>
#include <jni.h>
#include <transcribe.h>
#include <transcribe/vad.h>

#include <atomic>
#include <chrono>
#include <cstdio>
#include <memory>
#include <string>
#include <vector>

#define LOG_TAG "HexAsr"
#define LOGE(...) __android_log_print(ANDROID_LOG_ERROR, LOG_TAG, __VA_ARGS__)

namespace {

struct Engine {
    transcribe_session *session = nullptr;
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
    if (value == nullptr) return {};
    const char *chars = env->GetStringUTFChars(value, nullptr);
    std::string out(chars);
    env->ReleaseStringUTFChars(value, chars);
    return out;
}

void throw_engine(JNIEnv *env, const std::string &message) {
    jclass type = env->FindClass("anomaly/hex/android/nativeasr/AsrException");
    if (type != nullptr) env->ThrowNew(type, message.c_str());
}

bool read_pcm(const std::string &path, std::vector<float> &out) {
    FILE *file = std::fopen(path.c_str(), "rb");
    if (file == nullptr) return false;
    std::fseek(file, 0, SEEK_END);
    const long bytes = std::ftell(file);
    std::fseek(file, 0, SEEK_SET);
    if (bytes < 0) {
        std::fclose(file);
        return false;
    }
    std::vector<int16_t> pcm(static_cast<size_t>(bytes) / 2);
    const size_t read = std::fread(pcm.data(), 2, pcm.size(), file);
    std::fclose(file);
    out.resize(read);
    for (size_t i = 0; i < read; i++) out[i] = pcm[i] / 32768.0f;
    return true;
}

}  // namespace

extern "C" JNIEXPORT jlong JNICALL
Java_anomaly_hex_android_nativeasr_NativeAsr_nativeOpen(JNIEnv *env, jclass, jstring path, jint threads) {
    transcribe_model_load_params load;
    transcribe_model_load_params_init(&load);
    transcribe_session_params session;
    transcribe_session_params_init(&session);
    session.n_threads = threads;
    auto engine = std::make_unique<Engine>();
    const std::string model = to_string(env, path);
    const transcribe_status status = transcribe_open(model.c_str(), &load, &session, &engine->session);
    if (status != TRANSCRIBE_OK) {
        throw_engine(env, std::string("Could not load model: ") + transcribe_status_string(status));
        return 0;
    }
    transcribe_set_abort_callback(engine->session, should_abort, engine.get());
    return reinterpret_cast<jlong>(engine.release());
}

extern "C" JNIEXPORT jstring JNICALL
Java_anomaly_hex_android_nativeasr_NativeAsr_nativeTranscribe(
    JNIEnv *env, jclass, jlong handle, jstring pcm_path, jstring language, jstring prompt, jlong deadline_ms) {
    auto *engine = reinterpret_cast<Engine *>(handle);
    engine->cancelled = false;
    engine->deadline_ms = deadline_ms > 0 ? now_ms() + deadline_ms : 0;

    std::vector<float> samples;
    if (!read_pcm(to_string(env, pcm_path), samples)) {
        throw_engine(env, "Could not read the recorded audio");
        return nullptr;
    }

    transcribe_run_params params;
    transcribe_run_params_init(&params);
    const std::string lang = to_string(env, language);
    const std::string context = to_string(env, prompt);
    const transcribe_model *model = transcribe_get_model(engine->session);
    transcribe_capabilities capabilities;
    transcribe_capabilities_init(&capabilities);
    transcribe_model_get_capabilities(model, &capabilities);
    if (!lang.empty() && capabilities.n_languages > 1) params.language = lang.c_str();
    // The dictionary reaches Whisper as a context prompt, as on desktop. Others ignore it.
    if (!context.empty() && transcribe_model_supports(model, TRANSCRIBE_FEATURE_CONTEXT_PROMPT)) {
        params.prompt = context.c_str();
    }

    const transcribe_status status = transcribe_run(engine->session, samples.data(), static_cast<int>(samples.size()), &params);
    if (status != TRANSCRIBE_OK) {
        if (status == TRANSCRIBE_ERR_ABORTED) {
            const bool deadline_passed = !engine->cancelled.load();
            throw_engine(env, deadline_passed ? "deadline" : "cancelled");
        } else {
            throw_engine(env, std::string("Transcription failed: ") + transcribe_status_string(status));
        }
        return nullptr;
    }
    return env->NewStringUTF(transcribe_full_text(engine->session));
}

// Silero, set up as on desktop: 0.4 hears a quiet whisper, and 160 ms keeps a short "Ok".
// Returns 1 for speech, 0 for none, and -1 when the detector can't run, so the caller keeps
// its level check alone rather than dropping a dictation.
extern "C" JNIEXPORT jint JNICALL
Java_anomaly_hex_android_nativeasr_NativeAsr_nativeHasSpeech(JNIEnv *env, jclass, jstring detector_path, jstring pcm_path) {
    std::vector<float> samples;
    if (!read_pcm(to_string(env, pcm_path), samples)) return -1;
    transcribe_model_load_params load;
    transcribe_model_load_params_init(&load);
    transcribe_model *model = nullptr;
    if (transcribe_model_load_file(to_string(env, detector_path).c_str(), &load, &model) != TRANSCRIBE_OK) return -1;
    transcribe_vad_session_params session_params;
    transcribe_vad_session_params_init(&session_params);
    session_params.n_threads = 1;
    transcribe_vad_session *session = nullptr;
    jint verdict = -1;
    if (transcribe_vad_session_init(model, &session_params, &session) == TRANSCRIBE_OK) {
        transcribe_vad_params params;
        transcribe_vad_params_init(&params);
        params.threshold = 0.4;
        params.min_speech_ms = 160;
        if (transcribe_vad_run(session, samples.data(), static_cast<int>(samples.size()), &params) == TRANSCRIBE_OK) {
            transcribe_vad_result result;
            transcribe_vad_result_init(&result);
            if (transcribe_vad_get_result(session, &result) == TRANSCRIBE_OK) verdict = result.n_segments > 0 ? 1 : 0;
        }
        transcribe_vad_session_free(session);
    }
    transcribe_model_free(model);
    return verdict;
}

extern "C" JNIEXPORT void JNICALL
Java_anomaly_hex_android_nativeasr_NativeAsr_nativeCancel(JNIEnv *, jclass, jlong handle) {
    if (handle != 0) reinterpret_cast<Engine *>(handle)->cancelled = true;
}

extern "C" JNIEXPORT void JNICALL
Java_anomaly_hex_android_nativeasr_NativeAsr_nativeClose(JNIEnv *, jclass, jlong handle) {
    if (handle == 0) return;
    auto *engine = reinterpret_cast<Engine *>(handle);
    transcribe_close(engine->session);
    delete engine;
}
