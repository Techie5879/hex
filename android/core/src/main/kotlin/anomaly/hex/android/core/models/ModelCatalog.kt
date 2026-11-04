package anomaly.hex.android.core.models

/** A downloadable model at a pinned revision and digest. None ship in the APK. */
data class ModelArtifact(
    val id: String,
    val name: String,
    val note: String,
    val url: String,
    val sha256: String,
    val sizeBytes: Long,
)

object ModelCatalog {
    private fun hf(repo: String, rev: String, file: String) = "https://huggingface.co/$repo/resolve/$rev/$file"

    val SPEECH = listOf(
        ModelArtifact(
            "whisper-tiny.en-q8", "Whisper tiny.en Q8", "Fastest. Weakest on names.",
            hf("handy-computer/whisper-tiny.en-gguf", "9dd46e00ef0577e354e658c07b3ac5580e9cd211", "whisper-tiny.en-Q8_0.gguf"),
            "e8c9b73c06344307d8b346e07fbe93dd88d894627854bcff31523f1ce44394fa", 45_904_544,
        ),
        ModelArtifact(
            "whisper-base.en-q8", "Whisper base.en Q8", "Good starting point.",
            hf("handy-computer/whisper-base.en-gguf", "0dd374130837de419035c77d76440aad61ca135c", "whisper-base.en-Q8_0.gguf"),
            "3b46ca40bccbf7609c68d88a36d96077a04ca7c87f2060ede06f129fac3e7652", 84_886_208,
        ),
        ModelArtifact(
            "whisper-small.en-q5", "Whisper small.en Q5", "Better accuracy, slower.",
            hf("handy-computer/whisper-small.en-gguf", "ab86448a84fcc15a96a9a2216d93a8dac8427da3", "whisper-small.en-Q5_K_M.gguf"),
            "e96a679e57d36f80bb26512f3893185d219ddd8880cdac4c433e5af13a001b47", 193_672_256,
        ),
        ModelArtifact(
            "parakeet-tdt-ctc-110m-q8", "Parakeet TDT-CTC 110M Q8", "Scales with clip length. No vocabulary prompt.",
            hf("handy-computer/parakeet-tdt_ctc-110m-gguf", "1c3f0d5378fdd358905cc410a93ed9e64f52d170", "parakeet-tdt_ctc-110m-Q8_0.gguf"),
            "7dd44c74a331d788a4e5f8b16913b3feb29ced22cf5613aad0e0f6cd30516296", 135_373_280,
        ),
        ModelArtifact(
            "moonshine-base-q8", "Moonshine base Q8", "Scales with clip length. No vocabulary prompt.",
            hf("handy-computer/moonshine-base-gguf", "124f8cf8d3c1c56ad0f6ebc44aaafc0622e62d97", "moonshine-base-Q8_0.gguf"),
            "7f0027dfd857d310b63a85ef57cadf183da712cc374f85a648f8bc18aaa2efc8", 77_476_480,
        ),
        ModelArtifact(
            "whisper-large-v3-turbo-q5", "Whisper large-v3-turbo Q5", "Desktop's family. Large; benchmark before relying on it.",
            hf("handy-computer/whisper-large-v3-turbo-gguf", "18ada752005f5fddef499256f9a458d0b37a203a", "whisper-large-v3-turbo-Q5_K_M.gguf"),
            "06fc9923c09fb3ae878ea12c34d1be3ed18fad039ab14b4b045c7b6c40af56b2", 619_628_096,
        ),
    )

    val CLEANUP = listOf(
        ModelArtifact(
            "speakoflow-mini-q4", "SpeakoFlow Mini Q4", "Local cleanup model the desktop uses.",
            hf("SpeakoFlow/speakoflow-mini", "835431771f72820251fe6c6b4b07f12b000e2647", "SpeakoFlow-Mini-0.8B-Q4_K_M.gguf"),
            "530e0a5d4e9314e3bae2c92aecf3ab39ca48e8cfecac41372058f9fcb742329d", 541_903_616,
        ),
    )

    val ALL = SPEECH + CLEANUP
    val RECOMMENDED_SPEECH = "whisper-base.en-q8"

    fun byId(id: String) = ALL.firstOrNull { it.id == id }
}
