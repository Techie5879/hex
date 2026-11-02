package anomaly.hex.android.data

import java.io.File
import java.io.FileOutputStream

/** Whole-file replacement that a crash leaves either old or new, never half-written. */
object AtomicFiles {
    fun write(target: File, bytes: ByteArray) {
        target.parentFile?.mkdirs()
        val temporary = File(target.parentFile, "${target.name}.tmp")
        FileOutputStream(temporary).use { out ->
            out.write(bytes)
            out.flush()
            out.fd.sync()
        }
        if (!temporary.renameTo(target)) {
            temporary.delete()
            throw java.io.IOException("Could not replace ${target.name}")
        }
    }
}
