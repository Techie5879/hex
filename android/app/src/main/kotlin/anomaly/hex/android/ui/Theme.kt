package anomaly.hex.android.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/** Hex's OpenCode-derived tokens: cool neutrals, one contrast colour, status colours for states only. */
@Immutable
data class HexColors(
    val deep: Color,
    val card: Color,
    val raised: Color,
    val text: Color,
    val muted: Color,
    val faint: Color,
    val border: Color,
    val contrast: Color,
    val onContrast: Color,
    val danger: Color,
    val success: Color,
    val hot: Color,
)

private val Dark = HexColors(
    deep = Color(0xFF0A0B0D),
    card = Color(0xFF17191D),
    raised = Color(0xFF202329),
    text = Color(0xF0FFFFFF),
    muted = Color(0x8FFFFFFF),
    faint = Color(0x5CFFFFFF),
    border = Color(0x1CFFFFFF),
    contrast = Color(0xF5FFFFFF),
    onContrast = Color(0xFF0A0B0D),
    danger = Color(0xFFE54D42),
    success = Color(0xFF34C759),
    hot = Color(0xFFFF9500),
)

private val Light = HexColors(
    deep = Color(0xFFF4F4F5),
    card = Color(0xFFFFFFFF),
    raised = Color(0xFFECECEE),
    text = Color(0xF00A0B0D),
    muted = Color(0x990A0B0D),
    faint = Color(0x660A0B0D),
    border = Color(0x1A0A0B0D),
    contrast = Color(0xFF111316),
    onContrast = Color(0xFFFFFFFF),
    danger = Color(0xFFC9372C),
    success = Color(0xFF1E9E43),
    hot = Color(0xFFC96F00),
)

val LocalHex = staticCompositionLocalOf { Light }

private fun scheme(c: HexColors, dark: Boolean): ColorScheme {
    val base = if (dark) darkColorScheme() else lightColorScheme()
    return base.copy(
        primary = c.contrast,
        onPrimary = c.onContrast,
        secondary = c.raised,
        onSecondary = c.text,
        secondaryContainer = c.raised,
        onSecondaryContainer = c.text,
        tertiary = c.raised,
        background = c.deep,
        onBackground = c.text,
        surface = c.card,
        onSurface = c.text,
        surfaceVariant = c.raised,
        onSurfaceVariant = c.muted,
        surfaceContainer = c.card,
        surfaceContainerHigh = c.card,
        surfaceContainerHighest = c.raised,
        surfaceContainerLow = c.card,
        outline = c.border,
        outlineVariant = c.border,
        error = c.danger,
    )
}

@Composable
fun HexTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val colors = if (dark) Dark else Light
    androidx.compose.runtime.CompositionLocalProvider(LocalHex provides colors) {
        MaterialTheme(
            colorScheme = scheme(colors, dark),
            shapes = Shapes(
                small = RoundedCornerShape(10.dp),
                medium = RoundedCornerShape(14.dp),
                large = RoundedCornerShape(18.dp),
                extraLarge = RoundedCornerShape(24.dp),
            ),
            typography = Typography(
                headlineLarge = TextStyle(fontSize = 30.sp, lineHeight = 36.sp, fontWeight = FontWeight.SemiBold),
                headlineSmall = TextStyle(fontSize = 22.sp, lineHeight = 28.sp, fontWeight = FontWeight.SemiBold),
                titleMedium = TextStyle(fontSize = 16.sp, lineHeight = 22.sp, fontWeight = FontWeight.Medium),
                bodyLarge = TextStyle(fontSize = 16.sp, lineHeight = 23.sp),
                bodyMedium = TextStyle(fontSize = 14.sp, lineHeight = 20.sp),
                labelSmall = TextStyle(fontSize = 12.sp, lineHeight = 16.sp, fontWeight = FontWeight.Medium),
            ),
            content = content,
        )
    }
}
