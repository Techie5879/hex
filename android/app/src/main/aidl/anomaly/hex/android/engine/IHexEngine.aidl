package anomaly.hex.android.engine;

/** The inference process. Calls block until done; cancel() interrupts from another thread. */
interface IHexEngine {
    /** Loads the speech model ahead of use, so a dictation's end isn't spent loading. */
    oneway void prewarm(String modelPath, int threads);
    /** 1 for speech, 0 for none, -1 when the detector can't run. */
    int hasSpeech(String detectorPath, String pcmPath);
    String transcribe(String pcmPath, String modelPath, String language, String prompt, int threads, long deadlineMs);
    String cleanup(String modelPath, String system, String transcript, int threads, long deadlineMs);
    void cancel();
    /** Frees loaded models. */
    void release();
    /** JSON: peak and current resident memory, and the loaded models. */
    String stats();
}
