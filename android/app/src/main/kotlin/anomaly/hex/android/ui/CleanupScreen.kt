package anomaly.hex.android.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import anomaly.hex.android.HexApp
import anomaly.hex.android.core.models.ModelCatalog
import anomaly.hex.android.core.profile.CleanupPolicy
import anomaly.hex.android.core.profile.ProfileKeys
import kotlinx.serialization.json.JsonPrimitive

@Composable
fun CleanupScreen(openPage: (String) -> Unit) {
    val app = HexApp.get(LocalContext.current)
    val profile by app.profile.flow.collectAsStateWithLifecycle()
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    val policy = CleanupPolicy.from(profile)
    val polishReady by app.chatGpt.signedIn.collectAsStateWithLifecycle()

    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
        Text("Cleanup", style = MaterialTheme.typography.headlineSmall, color = hex.text, modifier = Modifier.padding(vertical = 16.dp))
        Note("Each step keeps the text if it fails. Cleanup aims to keep every name, number, and qualifier, but no model is perfect, so read important text before sending it.")

        if (profile.openConflicts.isNotEmpty()) {
            SectionLabel("Needs a decision")
            Card {
                for (conflict in profile.openConflicts) {
                    Row(
                        "Two devices changed: ${ProfileKeys.label(conflict.key)}",
                        "This device now uses the newer one. Choose which to keep.",
                        onClick = { openPage("settings/conflicts") },
                    )
                }
            }
        }

        SectionLabel("On this phone")
        Card {
            val name = settings.cleanupModel?.takeIf(app.models::isInstalled)?.let(ModelCatalog::byId)?.name
            Row("Local cleanup", name ?: "Removes um, uh, and similar, with no model", onClick = { openPage("settings/models") })
            Divider()
            PromptEditor("Local prompt", policy.localPrompt, CleanupPolicy.DEFAULT_LOCAL_PROMPT) {
                app.profile.putSetting(ProfileKeys.LOCAL_CLEANUP_PROMPT, JsonPrimitive(it))
            }
        }

        SectionLabel("Long dictations")
        Card {
            Row("ChatGPT sign-in", if (polishReady) "Signed in" else "Needed for polish", onClick = { openPage("settings/chatgpt") })
            Divider()
            SwitchRow(
                "Polish long text",
                if (polishReady) "Sent to ${policy.model.substringAfterLast('/')} with your ChatGPT sign-in" else "Sign in with ChatGPT first",
                checked = policy.polishEnabled,
                enabled = polishReady,
            ) { app.profile.putSetting(ProfileKeys.POLISH_ENABLED, JsonPrimitive(it)) }
            Divider()
            NumberRow("At least this many words", policy.minWords) { app.profile.putSetting(ProfileKeys.POLISH_MIN_WORDS, JsonPrimitive(it)) }
            Divider()
            NumberRow("Or this many characters", policy.minChars) { app.profile.putSetting(ProfileKeys.POLISH_MIN_CHARS, JsonPrimitive(it)) }
            Divider()
            NumberRow("Deadline in seconds", policy.deadlineSeconds) { app.profile.putSetting(ProfileKeys.POLISH_DEADLINE_SECONDS, JsonPrimitive(it)) }
            Divider()
            TextRow("Model", policy.model) { app.profile.putSetting(ProfileKeys.POLISH_MODEL, JsonPrimitive(it)) }
            Divider()
            TextRow("Reasoning", policy.variant.orEmpty()) { app.profile.putSetting(ProfileKeys.POLISH_VARIANT, JsonPrimitive(it)) }
            Divider()
            PromptEditor("Polish prompt", policy.polishPrompt, CleanupPolicy.DEFAULT_POLISH_PROMPT) {
                app.profile.putSetting(ProfileKeys.POLISH_PROMPT, JsonPrimitive(it))
            }
        }
        Note("Shorter dictations stay local, for speed. A timeout, an outage, or a used-up quota keeps the local text.")
        Spacer(Modifier.height(20.dp))
    }
}

@Composable
private fun PromptEditor(title: String, value: String, default: String, save: (String) -> Unit) {
    var editing by remember { mutableStateOf(false) }
    var draft by remember(value) { mutableStateOf(value) }
    if (!editing) {
        Row(title, value.take(120) + if (value.length > 120) "…" else "", onClick = { editing = true })
    } else {
        Column(Modifier.padding(16.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium, color = hex.text)
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(draft, { draft = it }, Modifier.fillMaxWidth().heightIn(min = 200.dp), colors = hexFieldColors())
            Text("${draft.length} characters. Saved whole, never shortened.", style = MaterialTheme.typography.labelSmall, color = hex.muted, modifier = Modifier.padding(top = 6.dp))
            Spacer(Modifier.height(10.dp))
            Row(horizontalArrangement = androidx.compose.foundation.layout.Arrangement.spacedBy(8.dp)) {
                SecondaryButton("Save") {
                    save(draft)
                    editing = false
                }
                SecondaryButton("Reset to default") { draft = default }
                SecondaryButton("Cancel") {
                    draft = value
                    editing = false
                }
            }
        }
    }
}

@Composable
private fun TextRow(title: String, value: String, save: (String) -> Unit) {
    var draft by remember(value) { mutableStateOf(value) }
    Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
        Text(title, style = MaterialTheme.typography.titleMedium, color = hex.text)
        Spacer(Modifier.height(6.dp))
        OutlinedTextField(
            draft,
            { draft = it; save(it.trim()) },
            Modifier.fillMaxWidth(),
            singleLine = true,
            colors = hexFieldColors(),
        )
    }
}
