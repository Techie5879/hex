package anomaly.hex.android.ui

import android.content.Context
import android.os.Build
import android.os.PowerManager
import anomaly.hex.android.HexApp
import anomaly.hex.android.core.models.ModelCatalog
import anomaly.hex.android.core.pipeline.CancelToken
import anomaly.hex.android.core.pipeline.CleanupPipeline
import anomaly.hex.android.core.profile.CleanupPolicy
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/** Measures installed models on this device. The results file stays private. */
object BenchmarkRunner {
    private const val CLIP_SECONDS_ASSET = "bench_en.pcm"
    private const val SAMPLE_TRANSCRIPT =
        "So um I wanted to say that the quarterly report for project nimbus is ready, and it's not final until Thursday, no, Friday. " +
            "We sold twelve hundred units, and I think the margin was around eighteen percent, but check that before you send it."

    private fun resultFile(context: Context) = File(context.noBackupFilesDir, "benchmark.json")

    fun run(context: Context, status: (String) -> Unit) {
        val app = HexApp.get(context)
        val clip = File(context.cacheDir, CLIP_SECONDS_ASSET)
        context.assets.open(CLIP_SECONDS_ASSET).use { input -> clip.outputStream().use { input.copyTo(it) } }
        val clipSeconds = clip.length() / 2 / 16000.0
        val thermal = context.getSystemService(PowerManager::class.java)
        val threads = app.settings.current.threads
        val rows = mutableListOf<kotlinx.serialization.json.JsonObject>()
        for (artifact in app.models.installedSpeech()) {
            status("Transcribing with ${artifact.name}")
            val path = app.models.file(artifact.id).path
            val times = mutableListOf<Long>()
            var text = ""
            repeat(3) {
                val started = System.nanoTime()
                text = app.engine.transcribe(clip.path, path, app.settings.current.language, "", threads, 0, CancelToken())
                times += (System.nanoTime() - started) / 1_000_000
            }
            rows += buildJsonObject {
                put("name", artifact.name)
                put("detail", "first ${times[0]} ms, then ${times.drop(1).minOrNull()} ms for ${"%.1f".format(clipSeconds)} s audio. " +
                    "${"%.1f".format(clipSeconds * 1000 / times.drop(1).min())}× realtime. " + memory(app) + " Thermal ${thermal.currentThermalStatus}. “${text.take(60)}”")
            }
        }
        for (artifact in ModelCatalog.CLEANUP.filter { app.models.isInstalled(it.id) }) {
            status("Cleaning up with ${artifact.name}")
            val path = app.models.file(artifact.id).path
            val cleaner = app.engine.cleaner(path, threads, 0)
            val system = CleanupPipeline.localSystem(CleanupPolicy.DEFAULT_LOCAL_PROMPT, emptyList())
            val times = mutableListOf<Long>()
            var text = ""
            repeat(2) {
                val started = System.nanoTime()
                text = cleaner.clean(system, SAMPLE_TRANSCRIPT, CancelToken())
                times += (System.nanoTime() - started) / 1_000_000
            }
            rows += buildJsonObject {
                put("name", artifact.name)
                put("detail", "first ${times[0]} ms, then ${times[1]} ms for ${SAMPLE_TRANSCRIPT.length} chars. " + memory(app) + " Thermal ${thermal.currentThermalStatus}. “${text.take(80)}”")
            }
        }
        val document = buildJsonObject {
            put("device", "${Build.MANUFACTURER} ${Build.MODEL}, Android ${Build.VERSION.RELEASE}")
            put("threads", threads)
            put("rows", JsonArray(rows))
        }
        resultFile(context).writeText(document.toString())
    }

    private fun memory(app: HexApp): String {
        val stats = app.engine.stats() ?: return ""
        val peak = runCatching { Json.parseToJsonElement(stats).jsonObject["peak_rss_kb"]?.jsonPrimitive?.content?.toLong() }.getOrNull() ?: return ""
        return "Peak engine memory ${peak / 1024} MB."
    }

    fun readResults(context: Context): List<Pair<String, String>> {
        val file = resultFile(context)
        if (!file.isFile) return emptyList()
        return runCatching {
            Json.parseToJsonElement(file.readText()).jsonObject.getValue("rows").jsonArray.map {
                val row = it.jsonObject
                row.getValue("name").jsonPrimitive.content to row.getValue("detail").jsonPrimitive.content
            }
        }.getOrDefault(emptyList())
    }
}
