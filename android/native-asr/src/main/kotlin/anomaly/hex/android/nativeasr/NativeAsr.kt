package anomaly.hex.android.nativeasr

class AsrException(message: String) : Exception(message)

/** One loaded speech model. Calls are serialized by the engine service. */
class NativeAsr private constructor(private val handle: Long, val modelPath: String) {
    fun transcribe(pcmPath: String, language: String, prompt: String, deadlineMs: Long): String =
        nativeTranscribe(handle, pcmPath, language, prompt, deadlineMs)

    fun cancel() = nativeCancel(handle)

    fun close() = nativeClose(handle)

    companion object {
        init {
            System.loadLibrary("hexasr")
        }

        fun open(modelPath: String, threads: Int) = NativeAsr(nativeOpen(modelPath, threads), modelPath)

        /** 1 for speech, 0 for none, -1 when the detector can't run. */
        fun hasSpeech(detectorPath: String, pcmPath: String): Int = nativeHasSpeech(detectorPath, pcmPath)

        @JvmStatic private external fun nativeOpen(path: String, threads: Int): Long
        @JvmStatic private external fun nativeTranscribe(handle: Long, pcmPath: String, language: String, prompt: String, deadlineMs: Long): String
        @JvmStatic private external fun nativeHasSpeech(detectorPath: String, pcmPath: String): Int
        @JvmStatic private external fun nativeCancel(handle: Long)
        @JvmStatic private external fun nativeClose(handle: Long)
    }
}
