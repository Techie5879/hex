package anomaly.hex.android.core.models

import kotlin.math.abs
import kotlin.math.log10
import kotlin.math.sqrt

/** The desktop's silence check: the loudest 30 ms window, in dBFS, against -50 dBFS. */
object AudioLevels {
    const val SILENCE_THRESHOLD_DBFS = -50f
    const val WINDOW_SAMPLES = 480
    const val MIN_DICTATION_SAMPLES = 16_000 * 3 / 10

    fun peak(samples: ShortArray, count: Int = samples.size): Float {
        var max = 0
        for (i in 0 until count) max = maxOf(max, abs(samples[i].toInt()))
        return max / 32768f
    }

    fun rmsDbfs(samples: ShortArray, start: Int, end: Int): Float {
        if (end <= start) return -120f
        var sum = 0.0
        for (i in start until end) {
            val s = samples[i] / 32768.0
            sum += s * s
        }
        val rms = sqrt(sum / (end - start))
        return if (rms <= 0.0) -120f else (20 * log10(rms)).toFloat()
    }
}
