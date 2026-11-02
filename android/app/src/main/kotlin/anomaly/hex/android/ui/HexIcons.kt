package anomaly.hex.android.ui

import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

/** The few tab icons Material's core set lacks. The extended set adds 40 MB of dex. */
object HexIcons {
    private fun outline(name: String, path: String) = ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f)
        .addPath(
            pathData = addPathNodes(path),
            stroke = SolidColor(androidx.compose.ui.graphics.Color.Black),
            strokeLineWidth = 1.8f,
            strokeLineCap = StrokeCap.Round,
            strokeLineJoin = StrokeJoin.Round,
        )
        .build()

    val Book = outline("Book", "M5 4.5h10a3 3 0 0 1 3 3V20H8a3 3 0 0 1-3-3ZM5 17a3 3 0 0 1 3-3h10M9 8.5h5")
    val Sparkle = outline("Sparkle", "M11 4l1.8 5.2L18 11l-5.2 1.8L11 18l-1.8-5.2L4 11l5.2-1.8ZM18 3.5v3M16.5 5h3M18 17v3M16.5 18.5h3")
    val Clock = outline("Clock", "M12 3.5a8.5 8.5 0 1 1 0 17a8.5 8.5 0 0 1 0-17ZM12 7.5V12l3 2")
}
