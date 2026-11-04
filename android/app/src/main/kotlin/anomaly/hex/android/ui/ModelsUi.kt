package anomaly.hex.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.RadioButtonDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import anomaly.hex.android.HexApp
import anomaly.hex.android.core.models.ModelArtifact
import anomaly.hex.android.core.models.ModelCatalog
import anomaly.hex.android.models.DownloadState
import anomaly.hex.android.models.ModelDownloadService

@Composable
fun SpeechModelList() {
    val app = HexApp.get(LocalContext.current)
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    Card {
        ModelCatalog.SPEECH.forEachIndexed { index, artifact ->
            ModelRow(artifact, settings.speechModel == artifact.id, artifact.id == ModelCatalog.RECOMMENDED_SPEECH) {
                app.settings.update { it.copy(speechModel = artifact.id) }
            }
            if (index < ModelCatalog.SPEECH.lastIndex) Divider()
        }
    }
}

@Composable
fun CleanupModelList() {
    val app = HexApp.get(LocalContext.current)
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    Card {
        Row(
            Modifier.fillMaxWidth().clickable { app.settings.update { it.copy(cleanupModel = null) } }.padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Selector(settings.cleanupModel == null) { app.settings.update { it.copy(cleanupModel = null) } }
            Column(Modifier.weight(1f)) {
                Text("Filler-word rules", style = MaterialTheme.typography.titleMedium, color = hex.text)
                Text("Instant. Removes um, uh, and similar only.", style = MaterialTheme.typography.bodyMedium, color = hex.muted)
            }
        }
        for (artifact in ModelCatalog.CLEANUP) {
            Divider()
            ModelRow(artifact, settings.cleanupModel == artifact.id, false) {
                app.settings.update { it.copy(cleanupModel = artifact.id) }
            }
        }
    }
}

@Composable
private fun Selector(selected: Boolean, enabled: Boolean = true, onClick: () -> Unit) {
    RadioButton(
        selected,
        onClick,
        enabled = enabled,
        colors = RadioButtonDefaults.colors(selectedColor = hex.text, unselectedColor = hex.faint, disabledUnselectedColor = hex.border),
    )
}

@Composable
private fun ModelRow(artifact: ModelArtifact, selected: Boolean, recommended: Boolean, select: () -> Unit) {
    val context = LocalContext.current
    val app = HexApp.get(context)
    val revision by app.models.revision.collectAsStateWithLifecycle()
    val state by app.models.state.collectAsStateWithLifecycle()
    revision.hashCode()
    val installed = app.models.isInstalled(artifact.id)
    val running = state as? DownloadState.Running
    val busy = running?.modelId == artifact.id || (state as? DownloadState.Verifying)?.modelId == artifact.id
    val anyBusy = state is DownloadState.Running || state is DownloadState.Verifying
    val failed = (state as? DownloadState.Failed)?.takeIf { it.modelId == artifact.id }
    val partial = app.models.partialBytes(artifact.id)

    Column(Modifier.fillMaxWidth().padding(start = 4.dp, end = 16.dp, top = 12.dp, bottom = 12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Selector(selected, enabled = installed, onClick = select)
            Column(Modifier.weight(1f)) {
                Text(artifact.name + if (recommended) " · Suggested" else "", style = MaterialTheme.typography.titleMedium, color = hex.text)
                Text(
                    "${formatSize(artifact.sizeBytes)} · ${artifact.note}",
                    style = MaterialTheme.typography.bodyMedium,
                    color = hex.muted,
                )
            }
        }
        Column(Modifier.padding(start = 48.dp)) {
            when {
                busy && running != null -> {
                    Spacer(Modifier.height(6.dp))
                    LinearProgressIndicator(
                        progress = { running.done.toFloat() / running.total },
                        Modifier.fillMaxWidth(),
                        color = hex.text,
                        trackColor = hex.raised,
                    )
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text("${formatSize(running.done)} of ${formatSize(running.total)}", style = MaterialTheme.typography.labelSmall, color = hex.muted, modifier = Modifier.weight(1f))
                        SecondaryButton("Pause") { app.models.cancel() }
                    }
                }
                busy -> Text("Checking the download…", style = MaterialTheme.typography.labelSmall, color = hex.muted, modifier = Modifier.padding(top = 6.dp))
                installed -> Row(Modifier.padding(top = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    SecondaryButton("Remove", enabled = !selected) { app.models.remove(artifact.id) }
                }
                else -> Row(Modifier.padding(top = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    SecondaryButton(if (partial > 0) "Resume (${formatSize(partial)} saved)" else "Download", enabled = !anyBusy) {
                        ModelDownloadService.start(context, artifact.id)
                    }
                }
            }
            failed?.let { Note(it.reason, hex.danger) }
        }
    }
}
