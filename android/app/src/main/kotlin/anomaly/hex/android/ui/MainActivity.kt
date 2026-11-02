package anomaly.hex.android.ui

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import anomaly.hex.android.HexApp

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        HexApp.get(this).sync.runNow()
        setContent { HexTheme { HexRoot() } }
    }

    companion object {
        fun pendingIntent(context: Context): PendingIntent = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
    }
}

enum class Tab(val label: String) { Home("Home"), Dictionary("Dictionary"), Cleanup("Cleanup"), History("History") }

@Composable
fun HexRoot() {
    val app = HexApp.get(LocalContext.current)
    val settings by app.settings.flow.collectAsStateWithLifecycle()
    var tab by rememberSaveable { mutableIntStateOf(0) }
    var settingsPage by rememberSaveable { mutableStateOf<String?>(null) }

    Box(Modifier.fillMaxSize().background(hex.deep)) {
        when {
            !settings.setupDone -> SetupFlow(onDone = { app.settings.update { it.copy(setupDone = true) } })
            settingsPage != null -> {
                BackHandler { settingsPage = settingsPage?.substringBeforeLast('/', "")?.ifEmpty { null } }
                SettingsScreens(
                    page = settingsPage!!,
                    open = { settingsPage = it },
                    close = { settingsPage = settingsPage?.substringBeforeLast('/', "")?.ifEmpty { null } },
                )
            }
            else -> Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.safeDrawing).imePadding()) {
                Box(Modifier.weight(1f)) {
                    when (Tab.entries[tab]) {
                        Tab.Home -> HomeScreen(openSettings = { settingsPage = "settings" }, openPage = { settingsPage = it })
                        Tab.Dictionary -> DictionaryScreen()
                        Tab.Cleanup -> CleanupScreen(openPage = { settingsPage = it })
                        Tab.History -> HistoryScreen()
                    }
                }
                NavigationBar(containerColor = hex.deep, tonalElevation = androidx.compose.ui.unit.Dp.Hairline) {
                    Tab.entries.forEachIndexed { index, item ->
                        NavigationBarItem(
                            selected = index == tab,
                            onClick = { tab = index },
                            label = { Text(item.label) },
                            icon = {
                                Icon(
                                    when (item) {
                                        Tab.Home -> Icons.Outlined.Home
                                        Tab.Dictionary -> HexIcons.Book
                                        Tab.Cleanup -> HexIcons.Sparkle
                                        Tab.History -> HexIcons.Clock
                                    },
                                    null,
                                )
                            },
                            colors = NavigationBarItemDefaults.colors(
                                selectedIconColor = hex.text,
                                selectedTextColor = hex.text,
                                indicatorColor = hex.raised,
                                unselectedIconColor = hex.muted,
                                unselectedTextColor = hex.muted,
                            ),
                        )
                    }
                }
            }
        }
    }
}
