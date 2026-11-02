package anomaly.hex.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
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
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import anomaly.hex.android.HexApp
import anomaly.hex.android.core.profile.ProfileException
import anomaly.hex.android.core.profile.VocabEntry

@Composable
fun hexFieldColors() = OutlinedTextFieldDefaults.colors(
    focusedBorderColor = hex.text,
    unfocusedBorderColor = hex.border,
    focusedContainerColor = hex.card,
    unfocusedContainerColor = hex.card,
    cursorColor = hex.text,
    focusedLabelColor = hex.muted,
    unfocusedLabelColor = hex.muted,
)

@Composable
fun DictionaryScreen() {
    val app = HexApp.get(LocalContext.current)
    val profile by app.profile.flow.collectAsStateWithLifecycle()
    var query by rememberSaveable { mutableStateOf("") }
    var adding by remember { mutableStateOf(false) }
    var editing by remember { mutableStateOf<VocabEntry?>(null) }
    val filtered = profile.liveTerms.filter {
        query.isBlank() || it.term.contains(query, true) || it.soundsLike.any { s -> s.contains(query, true) }
    }

    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
            Text("Dictionary", style = MaterialTheme.typography.headlineSmall, color = hex.text, modifier = Modifier.padding(vertical = 16.dp))
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
            if (filtered.isEmpty()) {
                Note(
                    if (profile.liveTerms.isEmpty()) "Add names and terms you use. Speech and cleanup read them as context. They correct a word only where the sentence fits, never by blind replacement."
                    else "No match.",
                )
            } else {
                Card(Modifier.weight(1f, fill = false)) {
                    LazyColumn {
                        items(filtered, key = { it.term }) { entry ->
                            Row(entry.term, entry.soundsLike.takeIf { it.isNotEmpty() }?.joinToString(", ")?.let { "May sound like: $it" }, onClick = { editing = entry }, trailing = {})
                            if (entry !== filtered.last()) Divider()
                        }
                    }
                }
            }
            Spacer(Modifier.height(88.dp))
        }
        FloatingActionButton(
            { adding = true },
            Modifier.align(Alignment.BottomEnd).padding(16.dp),
            containerColor = hex.contrast,
            contentColor = hex.onContrast,
        ) { Icon(Icons.Default.Add, "Add word") }
    }

    if (adding) TermDialog(null, onDismiss = { adding = false })
    editing?.let { entry -> TermDialog(entry, onDismiss = { editing = null }) }
}

@Composable
private fun TermDialog(entry: VocabEntry?, onDismiss: () -> Unit) {
    val app = HexApp.get(LocalContext.current)
    var term by remember { mutableStateOf(entry?.term ?: "") }
    var spellings by remember { mutableStateOf(entry?.soundsLike?.joinToString(", ") ?: "") }
    var error by remember { mutableStateOf<String?>(null) }
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = hex.card,
        title = { Text(if (entry == null) "Add word" else "Edit word", color = hex.text) },
        text = {
            Column {
                OutlinedTextField(term, { term = it; error = null }, Modifier.fillMaxWidth(), label = { Text("Word or name") }, singleLine = true, colors = hexFieldColors(), isError = error != null)
                Spacer(Modifier.height(10.dp))
                OutlinedTextField(spellings, { spellings = it }, Modifier.fillMaxWidth(), label = { Text("May sound like (optional)") }, colors = hexFieldColors())
                error?.let { Note(it, hex.danger) }
            }
        },
        confirmButton = {
            TextButton({
                try {
                    if (entry != null && termChanged(entry, term)) app.profile.removeTerm(entry.term)
                    app.profile.putTerm(term, spellings.split(',', ';', '\n'))
                    onDismiss()
                } catch (e: ProfileException) {
                    error = e.message
                }
            }) { Text(if (entry == null) "Add" else "Save", color = hex.text) }
        },
        dismissButton = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (entry != null) {
                    TextButton({
                        app.profile.removeTerm(entry.term)
                        onDismiss()
                    }) { Text("Delete", color = hex.danger) }
                }
                TextButton(onDismiss) { Text("Cancel", color = hex.muted) }
            }
        },
    )
}

private fun termChanged(entry: VocabEntry, text: String) =
    anomaly.hex.android.core.profile.termKey(entry.term) != anomaly.hex.android.core.profile.termKey(text)
