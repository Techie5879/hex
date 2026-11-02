package anomaly.hex.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import anomaly.hex.android.HexApp
import anomaly.hex.android.core.models.ModelCatalog

@Composable
fun HomeScreen(openSettings: () -> Unit, openPage: (String) -> Unit) {
    val context = LocalContext.current
    val app = HexApp.get(context)
    val readiness = rememberReadiness()
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    val modelRevision by app.models.revision.collectAsStateWithLifecycle()
    modelRevision.hashCode()
    val modelReady = settings.speechModel?.let(app.models::isInstalled) == true
    val ready = readiness.accessibility && readiness.microphone && modelReady

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
        Row(Modifier.fillMaxWidth().height(60.dp), verticalAlignment = Alignment.CenterVertically) {
            HexMark(24.dp)
            Spacer(Modifier.width(10.dp))
            Text("Hex", style = MaterialTheme.typography.headlineSmall, color = hex.text, modifier = Modifier.weight(1f))
            IconButton(openSettings) { Icon(Icons.Outlined.Settings, "Settings", tint = hex.text) }
        }
        Card {
            Row(
                title = if (ready) "Ready to dictate" else "Hex needs attention",
                detail = if (ready) "Tap any text field and use the button at the edge." else "Fix the items below to start dictating.",
            ) { StatusDot(if (ready) hex.success else hex.hot, 10.dp) }
            if (!readiness.accessibility) {
                Divider()
                Row("Turn on Hex in Accessibility", "The button can't show until this is on.", onClick = { openAccessibilitySettings(context) })
            }
            if (!readiness.microphone) {
                Divider()
                Row("Allow the microphone", "Open App info, then Permissions.", onClick = { openAppInfo(context) })
            }
            if (!modelReady) {
                Divider()
                Row("Get a speech model", "None is installed yet.", onClick = { openPage("settings/models") })
            }
        }
        SectionLabel("Try it")
        TryField()
        SectionLabel("Now using")
        Card {
            val speech = settings.speechModel?.let(ModelCatalog::byId)?.name ?: "None"
            val cleanup = settings.cleanupModel?.takeIf(app.models::isInstalled)?.let(ModelCatalog::byId)?.name ?: "Filler-word rules"
            Row("Speech", speech, onClick = { openPage("settings/models") })
            Divider()
            Row("Cleanup", cleanup, onClick = { openPage("settings/models") })
        }
        Spacer(Modifier.height(20.dp))
    }
}

/** A real text field, so the bubble's full loop can be tried inside Hex. */
@Composable
fun TryField() {
    var text by rememberSaveable { mutableStateOf("") }
    val colors = hex
    Box(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 120.dp)
            .background(colors.card, MaterialTheme.shapes.large)
            .border(1.dp, colors.border, MaterialTheme.shapes.large)
            .padding(16.dp),
    ) {
        if (text.isEmpty()) Text("Dictate something here", style = MaterialTheme.typography.bodyLarge, color = colors.faint)
        BasicTextField(
            text,
            { text = it },
            Modifier.fillMaxWidth().heightIn(min = 88.dp),
            textStyle = MaterialTheme.typography.bodyLarge.copy(color = colors.text),
            cursorBrush = SolidColor(colors.text),
        )
    }
}
