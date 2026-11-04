package anomaly.hex.android.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import anomaly.hex.android.core.codex.CodexTokenStore
import anomaly.hex.android.core.codex.CodexTokens
import java.io.IOException
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

/**
 * The sync token and this phone's ChatGPT login, wrapped by a hardware-backed
 * Keystore key where the device has one. Neither is in the APK, the profile,
 * backups, or logs.
 */
class SecureStore(context: Context) {
    private val prefs = context.getSharedPreferences("hex-secure", Context.MODE_PRIVATE)
    private val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    private fun key(): SecretKey {
        (keyStore.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    /** False if the change didn't reach the disk. */
    fun putToken(name: String, token: String?): Boolean {
        if (token.isNullOrEmpty()) return prefs.edit().remove(name).commit()
        val cipher = Cipher.getInstance(TRANSFORM).apply { init(Cipher.ENCRYPT_MODE, key()) }
        val sealed = cipher.iv + cipher.doFinal(token.toByteArray())
        return prefs.edit().putString(name, Base64.encodeToString(sealed, Base64.NO_WRAP)).commit()
    }

    fun token(name: String): String? {
        val stored = prefs.getString(name, null) ?: return null
        return runCatching {
            val bytes = Base64.decode(stored, Base64.NO_WRAP)
            val cipher = Cipher.getInstance(TRANSFORM).apply {
                init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, IV_BYTES))
            }
            String(cipher.doFinal(bytes, IV_BYTES, bytes.size - IV_BYTES))
        }.getOrNull()
    }

    fun has(name: String) = prefs.contains(name)

    companion object {
        private const val ALIAS = "hex-companion"
        private const val TRANSFORM = "AES/GCM/NoPadding"
        private const val IV_BYTES = 12
        const val SYNC_TOKEN = "sync"
        const val CODEX_TOKENS = "codex"
    }
}

/** Committed before use: a rotated refresh token that isn't saved ends the session. */
class SecureCodexTokens(private val secure: SecureStore) : CodexTokenStore {
    override fun load(): CodexTokens? =
        secure.token(SecureStore.CODEX_TOKENS)?.let { runCatching { Json.decodeFromString<CodexTokens>(it) }.getOrNull() }

    override fun save(tokens: CodexTokens?) {
        if (!secure.putToken(SecureStore.CODEX_TOKENS, tokens?.let { Json.encodeToString(it) })) throw IOException("Couldn't save the sign-in")
    }
}
