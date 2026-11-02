package anomaly.hex.android.data

import android.content.ContentValues
import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import anomaly.hex.android.core.pipeline.ProcessedText
import anomaly.hex.android.core.retention.Retention
import java.io.File
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

data class HistoryEntry(
    val id: Long,
    val createdMs: Long,
    val app: String,
    val raw: String,
    val text: String,
    val stages: String,
    val failures: String,
    val audioName: String?,
    /** Recovered audio that never got transcribed. */
    val pendingPcm: String? = null,
)

/** Local dictation text with its own retention, bounded like desktop's store. */
class HistoryStore(context: Context) {
    private val helper = object : SQLiteOpenHelper(context, File(context.noBackupFilesDir, "history.db").path, null, 1) {
        override fun onCreate(db: SQLiteDatabase) {
            db.execSQL(
                """CREATE TABLE history(
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    created INTEGER NOT NULL,
                    app TEXT NOT NULL,
                    raw TEXT NOT NULL,
                    final TEXT NOT NULL,
                    stages TEXT NOT NULL,
                    failures TEXT NOT NULL,
                    audio TEXT,
                    pending TEXT)""",
            )
            db.execSQL("CREATE INDEX history_created ON history(created)")
        }

        override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) = Unit
    }
    private val revisions = MutableStateFlow(0)
    val revision: StateFlow<Int> = revisions

    fun add(entry: HistoryEntry): Long {
        val values = ContentValues().apply {
            put("created", entry.createdMs)
            put("app", entry.app)
            put("raw", entry.raw)
            put("final", entry.text)
            put("stages", entry.stages)
            put("failures", entry.failures)
            put("audio", entry.audioName)
            put("pending", entry.pendingPcm)
        }
        val id = helper.writableDatabase.insertOrThrow("history", null, values)
        enforceCaps()
        revisions.value += 1
        return id
    }

    fun complete(id: Long, processed: ProcessedText, audioName: String? = null) {
        helper.writableDatabase.update(
            "history",
            ContentValues().apply {
                put("raw", processed.raw)
                put("final", processed.text)
                put("stages", processed.stageNames)
                put("failures", processed.failureNotes)
                if (audioName != null) put("audio", audioName)
                putNull("pending")
            },
            "id=?",
            arrayOf(id.toString()),
        )
        revisions.value += 1
    }

    fun list(query: String = "", limit: Int = 200): List<HistoryEntry> {
        val where = if (query.isBlank()) null else "raw LIKE ? ESCAPE '\\' OR final LIKE ? ESCAPE '\\'"
        val like = "%" + query.replace("\\", "\\\\").replace("%", "\\%").replace("_", "\\_") + "%"
        val args = if (query.isBlank()) null else arrayOf(like, like)
        helper.readableDatabase.query("history", null, where, args, null, null, "created DESC, id DESC", limit.toString()).use { c ->
            val out = ArrayList<HistoryEntry>(c.count)
            while (c.moveToNext()) {
                out += HistoryEntry(
                    id = c.getLong(0),
                    createdMs = c.getLong(1),
                    app = c.getString(2),
                    raw = c.getString(3),
                    text = c.getString(4),
                    stages = c.getString(5),
                    failures = c.getString(6),
                    audioName = c.getString(7),
                    pendingPcm = c.getString(8),
                )
            }
            return out
        }
    }

    fun delete(id: Long) {
        helper.writableDatabase.delete("history", "id=?", arrayOf(id.toString()))
        revisions.value += 1
    }

    fun clear() {
        helper.writableDatabase.delete("history", null, null)
        revisions.value += 1
    }

    /** Off and Forever delete nothing; a dated retention deletes what passed its age. */
    fun prune(retention: Retention, nowMs: Long = System.currentTimeMillis()): Int {
        val age = retention.maxAgeMs ?: return 0
        val removed = helper.writableDatabase.delete("history", "created<=? AND pending IS NULL", arrayOf((nowMs - age).toString()))
        if (removed > 0) revisions.value += 1
        return removed
    }

    /** Failed captures waiting in History expire with audio retention. Returns their file names. */
    fun prunePending(retention: Retention, nowMs: Long = System.currentTimeMillis()): List<String> {
        val age = retention.maxAgeMs ?: return emptyList()
        val expired = list(limit = MAX_ENTRIES).filter { it.pendingPcm != null && it.createdMs <= nowMs - age }
        for (entry in expired) delete(entry.id)
        return expired.mapNotNull { it.pendingPcm }
    }

    private fun enforceCaps() {
        helper.writableDatabase.execSQL(
            "DELETE FROM history WHERE id IN (SELECT id FROM history ORDER BY created DESC, id DESC LIMIT -1 OFFSET $MAX_ENTRIES)",
        )
    }

    private companion object {
        const val MAX_ENTRIES = 10_000
    }
}
