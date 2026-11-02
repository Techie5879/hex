package anomaly.hex.android.ui

import android.os.Build
import android.os.PowerManager
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import anomaly.hex.android.HexApp
import anomaly.hex.android.core.models.ModelCatalog
import anomaly.hex.android.core.profile.ProfileKeys
import anomaly.hex.android.core.retention.Retention
import anomaly.hex.android.data.DeviceSettings
import anomaly.hex.android.data.SecureStore
import anomaly.hex.android.service.HexFocus
import anomaly.hex.android.sync.SyncRunner
import java.text.DateFormat
import java.util.Date
import kotlin.concurrent.thread
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonPrimitive

@Composable
fun SettingsScreens(page: String, open: (String) -> Unit, close: () -> Unit) {
    val titles = mapOf(
        "settings" to "Settings",
        "settings/bubble-size" to "Button size",
        "settings/bubble-opacity" to "Button opacity",
        "settings/models" to "Models",
        "settings/benchmark" to "Benchmark",
        "settings/retention" to "Recordings and history",
        "settings/sync" to "Sync and polish",
        "settings/conflicts" to "Sync conflicts",
        "settings/exclusions" to "Apps without the button",
    )
    Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).imePadding()) {
        BackHeader(titles[page] ?: "Settings", close)
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
            when (page) {
                "settings" -> SettingsHome(open)
                "settings/bubble-size" -> BubbleSizePage()
                "settings/bubble-opacity" -> BubbleOpacityPage()
                "settings/models" -> ModelsPage(open)
                "settings/benchmark" -> BenchmarkPage()
                "settings/retention" -> RetentionPage()
                "settings/sync" -> SyncPage(open)
                "settings/conflicts" -> ConflictsPage()
                "settings/exclusions" -> ExclusionsPage()
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

@Composable
private fun SettingsHome(open: (String) -> Unit) {
    val context = LocalContext.current
    val app = HexApp.get(context)
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    SectionLabel("Button")
    Card {
        Row("Size", "${settings.bubbleSizeDp} dp", onClick = { open("settings/bubble-size") })
        Divider()
        Row("Opacity", "${settings.bubbleOpacity}%", onClick = { open("settings/bubble-opacity") })
        Divider()
        Row("Apps without the button", settings.excludedPackages.size.let { if (it == 0) "None" else "$it" }, onClick = { open("settings/exclusions") })
    }
    SectionLabel("Dictation")
    Card {
        Row("Models", settings.speechModel?.let(ModelCatalog::byId)?.name ?: "None", onClick = { open("settings/models") })
        Divider()
        SwitchRow("Smart spacing", "Adds a space when text joins a word", settings.smartSpacing) { on ->
            app.settings.update { it.copy(smartSpacing = on) }
        }
    }
    SectionLabel("Data")
    Card {
        Row("Recordings and history", "Recordings: ${settings.audioRetention.label()}. Text: ${settings.historyRetention.label()}.", onClick = { open("settings/retention") })
        Divider()
        Row("Sync and polish", if (settings.syncEnabled) "On" else "Off", onClick = { open("settings/sync") })
    }
    SectionLabel("Reliability")
    Card {
        Row("Battery", "If the button stops appearing, set Hex to Unrestricted.", onClick = { openBatterySettings(context) })
        Divider()
        Row("Accessibility", "Open system settings", onClick = { openAccessibilitySettings(context) })
    }
    Note("Hex for Android · Nothing is uploaded unless you turn on sync or polish.")
}

private fun Retention.label() = when (this) {
    Retention.Off -> "off"
    Retention.Day -> "1 day"
    Retention.Week -> "1 week"
    Retention.Month -> "1 month"
    Retention.Forever -> "forever"
}

@Composable
private fun BubblePreview(settings: DeviceSettings) {
    val plate = Color(0xFF17191D)
    val line = Color(0xF0FFFFFF)
    Box(Modifier.fillMaxWidth().height(160.dp), contentAlignment = Alignment.Center) {
        Canvas(Modifier.padding(8.dp).alpha(settings.bubbleOpacity / 100f).height(settings.bubbleSizeDp.dp).fillMaxWidth()) {
            val s = size.height
            val left = (size.width - s) / 2
            drawRoundRect(plate, Offset(left, 0f), Size(s, s), CornerRadius(s * 0.3f))
            val c = Offset(left + s / 2, s / 2)
            val r = s * 0.26f
            var previous: Offset? = null
            for (i in 0..6) {
                val a = PI / 3 * i - PI / 2
                val p = Offset(c.x + r * cos(a).toFloat(), c.y + r * sin(a).toFloat())
                previous?.let { drawLine(line, it, p, strokeWidth = s * 0.05f) }
                previous = p
            }
            drawCircle(line, s * 0.03f, c, style = Stroke(s * 0.03f))
        }
    }
}

@Composable
private fun HexSlider(value: Float, range: ClosedFloatingPointRange<Float>, steps: Int, onChange: (Float) -> Unit) {
    Slider(
        value,
        onChange,
        valueRange = range,
        steps = steps,
        colors = SliderDefaults.colors(thumbColor = hex.contrast, activeTrackColor = hex.contrast, inactiveTrackColor = hex.raised),
        modifier = Modifier.padding(horizontal = 16.dp),
    )
}

@Composable
private fun BubbleSizePage() {
    val app = HexApp.get(LocalContext.current)
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    BubblePreview(settings)
    Card {
        Spacer(Modifier.height(8.dp))
        HexSlider(settings.bubbleSizeDp.toFloat(), 28f..72f, 21) { v -> app.settings.update { it.copy(bubbleSizeDp = v.roundToInt()) } }
        Row(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
            Text("${settings.bubbleSizeDp} dp", style = MaterialTheme.typography.bodyMedium, color = hex.muted, modifier = Modifier.weight(1f))
            TextButton({ app.settings.update { it.copy(bubbleSizeDp = DeviceSettings.DEFAULT_SIZE_DP) } }) { Text("Reset", color = hex.text) }
        }
    }
    Note("The touch area stays at least 48 dp, whatever the size.")
    Card {
        SwitchRow("Shrink when not in use", checked = settings.shrinkWhenIdle) { on -> app.settings.update { it.copy(shrinkWhenIdle = on) } }
        Divider()
        SwitchRow("Shrink to a dot", checked = settings.shrinkToDot) { on -> app.settings.update { it.copy(shrinkToDot = on) } }
        Divider()
        SwitchRow("Shrink in search fields", checked = settings.shrinkInSearch) { on -> app.settings.update { it.copy(shrinkInSearch = on) } }
    }
}

@Composable
private fun BubbleOpacityPage() {
    val app = HexApp.get(LocalContext.current)
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    BubblePreview(settings)
    Card {
        Spacer(Modifier.height(8.dp))
        HexSlider(settings.bubbleOpacity.toFloat(), 20f..100f, 15) { v -> app.settings.update { it.copy(bubbleOpacity = v.roundToInt()) } }
        Row(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
            Text("${settings.bubbleOpacity}%", style = MaterialTheme.typography.bodyMedium, color = hex.muted, modifier = Modifier.weight(1f))
            TextButton({ app.settings.update { it.copy(bubbleOpacity = DeviceSettings.DEFAULT_OPACITY) } }) { Text("Reset", color = hex.text) }
        }
    }
    Note("It's fully opaque while it records or works.")
}

@Composable
private fun ModelsPage(open: (String) -> Unit) {
    val app = HexApp.get(LocalContext.current)
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    SectionLabel("Speech")
    SpeechModelList()
    SectionLabel("Local cleanup")
    CleanupModelList()
    Note("Whisper models read your dictionary as context. Parakeet and Moonshine ignore it here, though cleanup still uses it. Sizes are download size; each model needs about that much free RAM while it runs.")
    SectionLabel("Performance")
    Card {
        Row("Benchmark", "Measure speed and memory on this phone", onClick = { open("settings/benchmark") })
        Divider()
        Row("CPU threads", "${settings.threads}") {
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                for (n in listOf(2, 4, 6)) {
                    SecondaryButton("$n") { app.settings.update { it.copy(threads = n) } }
                }
            }
        }
        Divider()
        NumberRow("Local cleanup deadline, seconds", settings.localCleanupDeadlineSeconds, maxDigits = 3) { n -> app.settings.update { it.copy(localCleanupDeadlineSeconds = n.coerceIn(1, 120)) } }
        Divider()
        NumberRow("Free memory after idle minutes", settings.unloadIdleMinutes, maxDigits = 3) { n -> app.settings.update { it.copy(unloadIdleMinutes = n.coerceIn(1, 120)) } }
    }
}

@Composable
private fun BenchmarkPage() {
    val context = LocalContext.current
    val app = HexApp.get(context)
    var results by remember { mutableStateOf(BenchmarkRunner.readResults(context)) }
    var running by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf("") }
    val thermal = context.getSystemService(PowerManager::class.java).currentThermalStatus
    val installed = remember { app.models.installedSpeech() + ModelCatalog.CLEANUP.filter { app.models.isInstalled(it.id) } }

    Note("Runs each installed model over a fixed synthetic clip, three times, on this phone. Only real-phone numbers mean anything: an emulator can't tell you phone speed.")
    Card {
        Row("Device", "${Build.MANUFACTURER} ${Build.MODEL}, Android ${Build.VERSION.RELEASE}")
        Divider()
        Row("Thermal status now", thermalLabel(thermal))
    }
    Spacer(Modifier.height(12.dp))
    PrimaryButton(if (running) status.ifEmpty { "Running…" } else "Run benchmark", enabled = !running && installed.isNotEmpty()) {
        running = true
        thread(name = "hex-benchmark") {
            BenchmarkRunner.run(context) { status = it }
            results = BenchmarkRunner.readResults(context)
            running = false
        }
    }
    if (installed.isEmpty()) Note("Download a model first.")
    if (results.isNotEmpty()) {
        SectionLabel("Last run")
        Card {
            results.forEachIndexed { i, line ->
                Row(line.first, line.second)
                if (i < results.lastIndex) Divider()
            }
        }
    }
}

private fun thermalLabel(status: Int) = when (status) {
    PowerManager.THERMAL_STATUS_NONE -> "Normal"
    PowerManager.THERMAL_STATUS_LIGHT -> "Light"
    PowerManager.THERMAL_STATUS_MODERATE -> "Moderate"
    PowerManager.THERMAL_STATUS_SEVERE -> "Severe"
    PowerManager.THERMAL_STATUS_CRITICAL -> "Critical"
    else -> "Very hot"
}

@Composable
private fun RetentionPage() {
    val app = HexApp.get(LocalContext.current)
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    var audioStats by remember { mutableStateOf(app.archive.clipCount() to app.archive.totalBytes()) }
    var confirmClearAudio by remember { mutableStateOf(false) }
    var confirmClearHistory by remember { mutableStateOf(false) }

    SectionLabel("Recordings")
    Card {
        Row("Keep compressed audio", "${audioStats.first} clips, ${formatSize(audioStats.second)}. On this phone only.")
        RetentionPills(settings.audioRetention) { choice ->
            app.settings.update { it.copy(audioRetention = choice) }
            app.archive.prune(choice)
            audioStats = app.archive.clipCount() to app.archive.totalBytes()
        }
        Divider()
        Row(if (confirmClearAudio) "Delete every recording?" else "Delete all recordings", onClick = {
            if (confirmClearAudio) {
                app.archive.deleteAll()
                audioStats = 0 to 0L
                confirmClearAudio = false
            } else {
                confirmClearAudio = true
            }
        }, trailing = {})
    }
    Note("Off stops new recordings and keeps the ones you have.")
    SectionLabel("Text history")
    Card {
        Row("Keep what you dictated", "Independent of recordings.")
        RetentionPills(settings.historyRetention) { choice ->
            app.settings.update { it.copy(historyRetention = choice) }
            app.history.prune(choice)
        }
        Divider()
        Row(if (confirmClearHistory) "Clear all text history?" else "Clear text history", onClick = {
            if (confirmClearHistory) {
                app.history.clear()
                confirmClearHistory = false
            } else {
                confirmClearHistory = true
            }
        }, trailing = {})
    }
    Note("Fields that ask keyboards not to learn, such as incognito tabs, add nothing to either.")
}

@Composable
private fun SyncPage(open: (String) -> Unit) {
    val app = HexApp.get(LocalContext.current)
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    val running by app.sync.running.collectAsStateWithLifecycle()
    val profile by app.profile.flow.collectAsStateWithLifecycle()
    var url by remember { mutableStateOf(settings.syncUrl) }
    var polish by remember { mutableStateOf(settings.polishUrl) }
    var token by remember { mutableStateOf("") }
    var message by remember { mutableStateOf<String?>(null) }

    Note("Sync keeps your dictionary and cleanup prompts the same on your phone and computer. It carries nothing else: no audio, history, or passwords. Point it at your own Hex companion or any WebDAV file.")
    Card {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            OutlinedTextField(url, { url = it.trim() }, Modifier.fillMaxWidth(), label = { Text("Profile URL") }, singleLine = true, colors = hexFieldColors(), placeholder = { Text("https://…/v1/profile") })
            OutlinedTextField(polish, { polish = it.trim() }, Modifier.fillMaxWidth(), label = { Text("Polish URL (optional)") }, singleLine = true, colors = hexFieldColors(), placeholder = { Text("https://…/v1/generate") })
            OutlinedTextField(
                token,
                { token = it.trim() },
                Modifier.fillMaxWidth(),
                label = { Text(if (app.secure.has(SecureStore.SYNC_TOKEN)) "Token saved. Enter to replace." else "Token") },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                colors = hexFieldColors(),
            )
            message?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = hex.danger) }
            SecondaryButton("Save") {
                try {
                    if (url.isNotBlank()) SyncRunner.validateUrl(url)
                    if (polish.isNotBlank()) SyncRunner.validateUrl(polish)
                    if (token.isNotBlank()) app.secure.putToken(SecureStore.SYNC_TOKEN, token)
                    token = ""
                    message = null
                    app.settings.update { it.copy(syncUrl = url, polishUrl = polish) }
                } catch (e: IllegalArgumentException) {
                    message = e.message
                }
            }
        }
    }
    Spacer(Modifier.height(12.dp))
    Card {
        SwitchRow("Sync", "Edits made offline sync when you're back.", settings.syncEnabled, enabled = settings.syncUrl.isNotBlank()) { on ->
            app.settings.update { it.copy(syncEnabled = on) }
            if (on) app.sync.runNow()
        }
        Divider()
        val status = when {
            running -> "Syncing…"
            settings.lastSyncError != null -> settings.lastSyncError
            settings.lastSyncMs > 0 -> "Last synced ${DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(settings.lastSyncMs))}"
            else -> "Not synced yet"
        }
        Row("Status", status)
        Divider()
        Row("Sync now", onClick = { app.sync.runNow() }, trailing = {})
        if (profile.openConflicts.isNotEmpty()) {
            Divider()
            Row("Conflicts", "${profile.openConflicts.size} to review", onClick = { open("settings/conflicts") })
        }
    }
}

@Composable
private fun ConflictsPage() {
    val app = HexApp.get(LocalContext.current)
    val profile by app.profile.flow.collectAsStateWithLifecycle()
    if (profile.openConflicts.isEmpty()) Note("No conflicts.")
    for (conflict in profile.openConflicts) {
        val current = profile.settings[conflict.key]?.value
        val currentText = (current as? JsonPrimitive)?.content ?: current.toString()
        val otherText = runCatching { conflict.value.jsonPrimitive.content }.getOrDefault(conflict.value.toString())
        SectionLabel(ProfileKeys.label(conflict.key))
        Card {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("In use (newer)", style = MaterialTheme.typography.labelSmall, color = hex.muted)
                Text(currentText, style = MaterialTheme.typography.bodyMedium, color = hex.text)
                Text("Other device's version", style = MaterialTheme.typography.labelSmall, color = hex.muted)
                Text(otherText, style = MaterialTheme.typography.bodyMedium, color = hex.text)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    SecondaryButton("Keep in-use") { app.profile.resolve(conflict.id, useConflictValue = false) }
                    SecondaryButton("Use other") { app.profile.resolve(conflict.id, useConflictValue = true) }
                }
            }
        }
    }
}

@Composable
private fun ExclusionsPage() {
    val app = HexApp.get(LocalContext.current)
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    val recent = remember { HexFocus.recent().filter { it !in settings.excludedPackages } }
    Note("Password and payment fields already hide the button. Add banking apps or anything else here, and the button never shows in them.")
    if (settings.excludedPackages.isNotEmpty()) {
        SectionLabel("Hidden in")
        Card {
            settings.excludedPackages.sorted().forEachIndexed { i, pkg ->
                Row(pkg, null) {
                    SecondaryButton("Show") { app.settings.update { it.copy(excludedPackages = it.excludedPackages - pkg) } }
                }
                if (i < settings.excludedPackages.size - 1) Divider()
            }
        }
    }
    if (recent.isNotEmpty()) {
        SectionLabel("Recently used")
        Card {
            recent.forEachIndexed { i, pkg ->
                Row(pkg, null) {
                    SecondaryButton("Hide") { app.settings.update { it.copy(excludedPackages = it.excludedPackages + pkg) } }
                }
                if (i < recent.lastIndex) Divider()
            }
        }
    }
    var typed by remember { mutableStateOf("") }
    SectionLabel("Add by package name")
    Card {
        Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            OutlinedTextField(typed, { typed = it.trim() }, Modifier.weight(1f), singleLine = true, colors = hexFieldColors(), placeholder = { Text("com.example.bank") })
            SecondaryButton("Add", enabled = typed.isNotBlank()) {
                app.settings.update { it.copy(excludedPackages = it.excludedPackages + typed) }
                typed = ""
            }
        }
    }
}
