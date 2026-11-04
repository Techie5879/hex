package anomaly.hex.android.ui

import android.Manifest
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import anomaly.hex.android.HexApp
import anomaly.hex.android.core.retention.Retention

private const val STEPS = 6

/** First run. Each step shows real state, and Accessibility is asked for only after its disclosure. */
@Composable
fun SetupFlow(onDone: () -> Unit) {
    var step by rememberSaveable { mutableIntStateOf(0) }
    Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).padding(horizontal = 24.dp)) {
        Spacer(Modifier.height(20.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
            HexMark(26.dp)
            Spacer(Modifier.weight(1f))
            for (i in 0 until STEPS) {
                Box(Modifier.size(6.dp).background(if (i <= step) hex.text else hex.border, CircleShape))
            }
        }
        Spacer(Modifier.height(24.dp))
        when (step) {
            0 -> Disclosure(onAgree = { step = 1 })
            1 -> MicrophoneStep(next = { step = 2 })
            2 -> AccessibilityStep(next = { step = 3 })
            3 -> ModelStep(next = { step = 4 })
            4 -> RetentionStep(next = { step = 5 })
            else -> TryStep(onDone)
        }
    }
}

@Composable
private fun StepBody(title: String, body: String? = null, content: @Composable ColumnScope.() -> Unit) {
    Text(title, style = MaterialTheme.typography.headlineLarge, color = hex.text)
    if (body != null) {
        Spacer(Modifier.height(10.dp))
        Text(body, style = MaterialTheme.typography.bodyLarge, color = hex.muted)
    }
    Spacer(Modifier.height(20.dp))
    Column(content = content)
}

@Composable
private fun ColumnScope.Disclosure(onAgree: () -> Unit) {
    val app = HexApp.get(LocalContext.current)
    Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
        StepBody("Dictate in any app", "Hex adds a small button to text fields. Tap it or hold it, and speak. Speech is transcribed on this phone.") {
            Card {
                Disclose("What Accessibility lets Hex do", "See which field has focus and what kind it is, so the button shows only on ordinary text fields. Insert your dictation at the cursor.")
                Divider()
                Disclose("What Hex stays away from", "No button in password fields or on the lock screen, or in payment fields Hex can recognize. Android doesn't identify every payment field, so exclude your banking apps in Settings. Hex reads only what it needs around your cursor, and keeps nothing from your screen.")
                Divider()
                Disclose("When it records", "Only after you tap or hold the button. A notification and the status bar show it while it records. You can cancel at any time.")
                Divider()
                Disclose("What leaves this phone", "Nothing by default. Long-text polish is opt-in and goes to OpenAI with your ChatGPT sign-in. Dictionary sync is opt-in and goes only to a server you set up.")
            }
        }
    }
    Spacer(Modifier.height(12.dp))
    PrimaryButton("Agree and continue") {
        app.settings.update { it.copy(disclosureAccepted = true) }
        onAgree()
    }
    Spacer(Modifier.height(16.dp))
}

@Composable
private fun Disclose(title: String, body: String) {
    Column(Modifier.padding(16.dp)) {
        Text(title, style = MaterialTheme.typography.titleMedium, color = hex.text)
        Spacer(Modifier.height(4.dp))
        Text(body, style = MaterialTheme.typography.bodyMedium, color = hex.muted)
    }
}

@Composable
private fun ColumnScope.MicrophoneStep(next: () -> Unit) {
    val readiness = rememberReadiness()
    val request = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {}
    Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
        StepBody("Allow the microphone", "Hex opens the microphone only while you dictate, then closes it. Notifications show when it's on.") {
            Card {
                Row("Microphone", if (readiness.microphone) "Allowed" else "Needed to dictate") { StatusDot(if (readiness.microphone) hex.success else hex.hot) }
                Divider()
                Row("Notifications", if (readiness.notifications) "Allowed" else "Shows recording state and download progress") { StatusDot(if (readiness.notifications) hex.success else hex.faint) }
            }
        }
    }
    // Notifications are optional, so a refusal there doesn't block setup. The microphone is needed.
    if (readiness.microphone) {
        PrimaryButton("Continue", onClick = next)
    } else {
        PrimaryButton("Allow") {
            request.launch(arrayOf(Manifest.permission.RECORD_AUDIO, Manifest.permission.POST_NOTIFICATIONS))
        }
    }
    Spacer(Modifier.height(16.dp))
}

@Composable
private fun ColumnScope.AccessibilityStep(next: () -> Unit) {
    val readiness = rememberReadiness()
    val context = LocalContext.current
    Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
        StepBody("Turn on Hex in Accessibility", "This shows the button and lets Hex place text at your cursor.") {
            Card {
                Numbered(1, "Open Accessibility settings", "Find Hex under Downloaded apps or Installed apps.")
                Divider()
                Numbered(2, "Turn Hex on", "Leave the Hex shortcut off. You don't need it.")
            }
            Spacer(Modifier.height(14.dp))
            Card {
                Disclose(
                    "Is the switch greyed out?",
                    "Android restricts this setting for apps installed outside an app store. Open Hex's App info, tap ⋮, choose Allow restricted settings, then come back.",
                )
                Row(Modifier.padding(start = 16.dp, bottom = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    SecondaryButton("Open App info") { openAppInfo(context) }
                }
            }
            Spacer(Modifier.height(14.dp))
            Card {
                Row("Hex service", if (readiness.accessibility) "On" else "Off") { StatusDot(if (readiness.accessibility) hex.success else hex.hot) }
            }
        }
    }
    if (readiness.accessibility) {
        PrimaryButton("Continue", onClick = next)
    } else {
        PrimaryButton("Open Accessibility settings") { openAccessibilitySettings(context) }
    }
    Spacer(Modifier.height(16.dp))
}

@Composable
private fun Numbered(number: Int, title: String, body: String) {
    Row(Modifier.fillMaxWidth().padding(16.dp)) {
        Box(Modifier.size(24.dp).background(hex.raised, CircleShape), contentAlignment = Alignment.Center) {
            Text("$number", style = MaterialTheme.typography.labelSmall, color = hex.text)
        }
        Spacer(Modifier.size(12.dp))
        Column {
            Text(title, style = MaterialTheme.typography.titleMedium, color = hex.text)
            Text(body, style = MaterialTheme.typography.bodyMedium, color = hex.muted)
        }
    }
}

@Composable
private fun ColumnScope.ModelStep(next: () -> Unit) {
    val app = HexApp.get(LocalContext.current)
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    val installed by app.models.revision.collectAsStateWithLifecycle()
    LaunchedEffect(installed) {
        if (settings.speechModel == null) {
            app.models.installedSpeech().firstOrNull()?.let { first -> app.settings.update { it.copy(speechModel = first.id) } }
        }
    }
    Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
        StepBody("Get a speech model", "Models run on this phone and aren't part of the app. You can change or benchmark them later in Settings.") {
            SpeechModelList()
        }
    }
    PrimaryButton("Continue", enabled = settings.speechModel?.let(app.models::isInstalled) == true, onClick = next)
    Spacer(Modifier.height(16.dp))
}

@Composable
private fun ColumnScope.RetentionStep(next: () -> Unit) {
    val app = HexApp.get(LocalContext.current)
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
        StepBody("Keep your dictations here", "Recordings and text stay on this phone, and nothing uploads them. Change either one at any time in Settings.") {
            Card {
                Row("Recordings", "Compressed audio of each dictation")
                RetentionPills(settings.audioRetention) { choice -> app.settings.update { it.copy(audioRetention = choice) } }
                Divider()
                Row("Text history", "What you said and what was inserted")
                RetentionPills(settings.historyRetention) { choice -> app.settings.update { it.copy(historyRetention = choice) } }
            }
        }
    }
    PrimaryButton("Continue", onClick = next)
    Spacer(Modifier.height(16.dp))
}

@Composable
fun RetentionPills(selected: Retention, onSelect: (Retention) -> Unit) {
    val labels = mapOf(Retention.Off to "Off", Retention.Day to "Day", Retention.Week to "Week", Retention.Month to "Month", Retention.Forever to "Forever")
    Pills(Retention.entries.map { it.wire to labels.getValue(it) }, selected.wire) { onSelect(Retention.fromWire(it)) }
}

@Composable
private fun ColumnScope.TryStep(onDone: () -> Unit) {
    Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
        StepBody("You're set", "Tap the field below, and the Hex button appears at the screen's edge.") {
            TryField()
            Spacer(Modifier.height(14.dp))
            Card {
                Row("Tap", "Hands-free. Tap ✓ to insert, or ✕ to cancel.")
                Divider()
                Row("Hold", "Talk while holding, release to insert. Slide away to cancel.")
                Divider()
                Row("Drag", "Move the button anywhere along either edge.")
            }
        }
    }
    PrimaryButton("Start using Hex", onClick = onDone)
    Spacer(Modifier.height(16.dp))
}
