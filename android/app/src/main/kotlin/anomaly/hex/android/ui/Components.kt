package anomaly.hex.android.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.sin

val hex @Composable get() = LocalHex.current

@Composable
fun Card(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier
            .fillMaxWidth()
            .background(hex.card, MaterialTheme.shapes.large)
            .border(1.dp, hex.border, MaterialTheme.shapes.large),
        content = content,
    )
}

@Composable
fun SectionLabel(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.labelSmall,
        color = hex.muted,
        modifier = Modifier.padding(start = 6.dp, top = 18.dp, bottom = 8.dp),
    )
}

@Composable
fun Row(
    title: String,
    detail: String? = null,
    onClick: (() -> Unit)? = null,
    trailing: (@Composable () -> Unit)? = null,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .heightIn(min = 56.dp)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleMedium, color = hex.text)
            if (detail != null) Text(detail, style = MaterialTheme.typography.bodyMedium, color = hex.muted)
        }
        Spacer(Modifier.width(12.dp))
        when {
            trailing != null -> trailing()
            onClick != null -> Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null, tint = hex.faint)
        }
    }
}

@Composable
fun Divider() {
    Box(Modifier.fillMaxWidth().padding(start = 16.dp).height(1.dp).background(hex.border))
}

@Composable
fun SwitchRow(title: String, detail: String? = null, checked: Boolean, enabled: Boolean = true, onChange: (Boolean) -> Unit) {
    Row(title, detail, onClick = { if (enabled) onChange(!checked) }) {
        Switch(
            checked = checked,
            enabled = enabled,
            onCheckedChange = onChange,
            colors = SwitchDefaults.colors(
                checkedTrackColor = hex.contrast,
                checkedThumbColor = hex.onContrast,
                uncheckedTrackColor = hex.raised,
                uncheckedThumbColor = hex.muted,
                uncheckedBorderColor = hex.border,
            ),
        )
    }
}

@Composable
fun PrimaryButton(text: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) {
    Button(
        onClick,
        modifier.fillMaxWidth().height(52.dp),
        enabled = enabled,
        shape = MaterialTheme.shapes.medium,
        colors = ButtonDefaults.buttonColors(containerColor = hex.contrast, contentColor = hex.onContrast, disabledContainerColor = hex.raised, disabledContentColor = hex.faint),
    ) { Text(text, style = MaterialTheme.typography.titleMedium) }
}

@Composable
fun SecondaryButton(text: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) {
    OutlinedButton(
        onClick,
        modifier.heightIn(min = 44.dp),
        enabled = enabled,
        shape = MaterialTheme.shapes.medium,
        border = androidx.compose.foundation.BorderStroke(1.dp, hex.border),
        colors = ButtonDefaults.outlinedButtonColors(contentColor = hex.text),
    ) { Text(text, style = MaterialTheme.typography.bodyMedium) }
}

@Composable
fun StatusDot(color: androidx.compose.ui.graphics.Color, size: Dp = 8.dp) {
    Box(Modifier.size(size).background(color, CircleShape))
}

/** Hex's concentric hexagon mark. */
@Composable
fun HexMark(size: Dp, modifier: Modifier = Modifier) {
    val color = hex.text
    Canvas(modifier.size(size)) {
        fun hexagon(radius: Float) = Path().apply {
            for (i in 0..5) {
                val angle = PI / 3 * i - PI / 2
                val point = Offset(center.x + radius * cos(angle).toFloat(), center.y + radius * sin(angle).toFloat())
                if (i == 0) moveTo(point.x, point.y) else lineTo(point.x, point.y)
            }
            close()
        }
        val width = this.size.minDimension
        drawPath(hexagon(width * 0.48f), color, style = Stroke(width * 0.075f, join = StrokeJoin.Round))
        drawPath(hexagon(width * 0.24f), color)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BackHeader(title: String, onBack: () -> Unit, action: (@Composable () -> Unit)? = null) {
    Row(
        Modifier.fillMaxWidth().height(56.dp).padding(horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back", tint = hex.text) }
        Text(title, style = MaterialTheme.typography.titleMedium, color = hex.text, modifier = Modifier.weight(1f))
        action?.invoke()
    }
}

@Composable
fun NumberRow(title: String, value: Int, maxDigits: Int = 6, save: (Int) -> Unit) {
    var draft by remember(value) { mutableStateOf(value.toString()) }
    Row(title) {
        OutlinedTextField(
            draft,
            { next ->
                draft = next.filter(Char::isDigit).take(maxDigits)
                draft.toIntOrNull()?.let(save)
            },
            Modifier.fillMaxWidth(0.3f),
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            colors = hexFieldColors(),
        )
    }
}

@Composable
fun Note(text: String, color: androidx.compose.ui.graphics.Color = hex.muted) {
    Text(text, style = MaterialTheme.typography.bodyMedium, color = color, modifier = Modifier.padding(horizontal = 6.dp, vertical = 8.dp))
}

@Composable
fun Pills(options: List<Pair<String, String>>, selected: String, onSelect: (String) -> Unit) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        for ((value, label) in options) {
            val on = value == selected
            Box(
                Modifier
                    .weight(1f)
                    .heightIn(min = 40.dp)
                    .background(if (on) hex.contrast else hex.raised, RoundedCornerShape(10.dp))
                    .clickable(role = Role.RadioButton) { onSelect(value) },
                contentAlignment = Alignment.Center,
            ) { Text(label, style = MaterialTheme.typography.bodyMedium, color = if (on) hex.onContrast else hex.text) }
        }
    }
}
