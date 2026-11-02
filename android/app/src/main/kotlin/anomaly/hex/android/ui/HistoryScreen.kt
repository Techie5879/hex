package anomaly.hex.android.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.media.MediaPlayer
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import anomaly.hex.android.HexApp
import anomaly.hex.android.data.HistoryEntry
import java.text.DateFormat
import java.util.Date
import kotlin.concurrent.thread

@Composable
fun HistoryScreen() {
    val context = LocalContext.current
    val app = HexApp.get(context)
    val revision by app.history.revision.collectAsStateWithLifecycle()
    var query by rememberSaveable { mutableStateOf("") }
    val entries = remember(revision, query) { app.history.list(query) }
    var expanded by remember { mutableLongStateOf(-1L) }
    var player by remember { mutableStateOf<MediaPlayer?>(null) }
    DisposableEffect(Unit) { onDispose { player?.release() } }

    Column(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
        Text("History", style = MaterialTheme.typography.headlineSmall, color = hex.text, modifier = Modifier.padding(vertical = 16.dp))
        OutlinedTextField(
            query,
            { query = it },
            Modifier.fillMaxWidth(),
            placeholder = { Text("Search") },
            leadingIcon = { Icon(Icons.Default.Search, null, tint = hex.faint) },
            singleLine = true,
            shape = MaterialTheme.shapes.medium,
            colors = hexFieldColors(),
        )
        Spacer(Modifier.height(12.dp))
        if (entries.isEmpty()) {
            Note(if (query.isBlank()) "Your dictations appear here, on this phone only." else "No match.")
        } else {
            Card(Modifier.weight(1f, fill = false)) {
                LazyColumn {
                    items(entries, key = { it.id }) { entry ->
                        HistoryRow(
                            entry,
                            expanded == entry.id,
                            onToggle = { expanded = if (expanded == entry.id) -1 else entry.id },
                            onPlay = { name ->
                                player?.release()
                                player = app.archive.clip(name)?.let { file ->
                                    MediaPlayer().apply {
                                        setDataSource(file.path)
                                        prepare()
                                        start()
                                    }
                                }
                            },
                            onCopy = {
                                val clip = ClipData.newPlainText("Hex dictation", entry.text)
                                context.getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
                            },
                            onDelete = {
                                entry.audioName?.let(app.archive::delete)
                                app.history.delete(entry.id)
                            },
                        )
                        if (entry !== entries.last()) Divider()
                    }
                }
            }
        }
        Spacer(Modifier.height(12.dp))
    }
}

@Composable
private fun HistoryRow(entry: HistoryEntry, expanded: Boolean, onToggle: () -> Unit, onPlay: (String) -> Unit, onCopy: () -> Unit, onDelete: () -> Unit) {
    val context = LocalContext.current
    val app = HexApp.get(context)
    Column(Modifier.fillMaxWidth().clickable(onClick = onToggle).padding(16.dp)) {
        Text(
            "${DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(entry.createdMs))} · ${entry.app}",
            style = MaterialTheme.typography.labelSmall,
            color = hex.muted,
        )
        Spacer(Modifier.height(4.dp))
        if (entry.pendingPcm != null) {
            Text("Recording saved, not transcribed yet.", style = MaterialTheme.typography.bodyLarge, color = hex.text)
            Spacer(Modifier.height(8.dp))
            PendingActions(entry)
            return@Column
        }
        Text(entry.text, style = MaterialTheme.typography.bodyLarge, color = hex.text, maxLines = if (expanded) Int.MAX_VALUE else 3)
        if (expanded) {
            if (entry.raw != entry.text) {
                Spacer(Modifier.height(8.dp))
                Text("As heard", style = MaterialTheme.typography.labelSmall, color = hex.muted)
                Text(entry.raw, style = MaterialTheme.typography.bodyMedium, color = hex.muted)
            }
            if (entry.stages.isNotEmpty()) {
                Spacer(Modifier.height(8.dp))
                Text("Steps: ${entry.stages}", style = MaterialTheme.typography.labelSmall, color = hex.muted)
            }
            if (entry.failures.isNotEmpty()) {
                Text("Kept the earlier text: ${entry.failures}", style = MaterialTheme.typography.labelSmall, color = hex.hot)
            }
            Spacer(Modifier.height(10.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                SecondaryButton("Copy", onClick = onCopy)
                entry.audioName?.takeIf { app.archive.clip(it) != null }?.let { name -> SecondaryButton("Play") { onPlay(name) } }
                SecondaryButton("Delete", onClick = onDelete)
            }
        }
    }
}

@Composable
private fun PendingActions(entry: HistoryEntry) {
    val app = HexApp.get(LocalContext.current)
    var working by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        SecondaryButton(if (working) "Transcribing…" else "Transcribe", enabled = !working) {
            working = true
            error = null
            thread(name = "hex-transcribe-pending") {
                error = PendingTranscriber.run(app, entry)
                working = false
            }
        }
        SecondaryButton("Delete", enabled = !working) {
            PendingTranscriber.delete(app, entry)
        }
    }
    error?.let { Note(it, hex.danger) }
}
