package anomaly.hex.android.nativellm

class LlmException(message: String) : Exception(message)

/** One loaded cleanup model. Calls are serialized by the engine service. */
class NativeLlm private constructor(private val handle: Long, val modelPath: String) {
    fun clean(system: String, transcript: String, deadlineMs: Long): String {
        val prompt = "<|im_start|>system\n$system<|im_end|>\n<|im_start|>user\n$transcript<|im_end|>\n" +
            "<|im_start|>assistant\n<think>\n\n</think>\n\n"
        val chars = transcript.codePointCount(0, transcript.length)
        val maxTokens = (Math.ceil(chars * 0.8).toInt() + 64).coerceIn(128, 3072)
        return nativeComplete(handle, prompt, maxTokens, deadlineMs)
    }

    fun cancel() = nativeCancel(handle)

    fun close() = nativeClose(handle)

    companion object {
        init {
            System.loadLibrary("hexllm")
        }

        fun open(modelPath: String, threads: Int, contextTokens: Int = 4096) =
            NativeLlm(nativeOpen(modelPath, threads, contextTokens), modelPath)

        @JvmStatic private external fun nativeOpen(path: String, threads: Int, nCtx: Int): Long
        @JvmStatic private external fun nativeComplete(handle: Long, prompt: String, maxTokens: Int, deadlineMs: Long): String
        @JvmStatic private external fun nativeCancel(handle: Long)
        @JvmStatic private external fun nativeClose(handle: Long)
    }
}
