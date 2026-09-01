package com.grafyn.app

import java.nio.charset.StandardCharsets
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

internal object GrafynAndroidContracts {
    const val KEY_ALIAS = "com.grafyn.app.secrets.aes-gcm.v1"
    const val SECRET_SERVICE = "com.grafyn.app"
    const val SHARE_DIRECTORY = "Grafyn/grafyn-share-v1"
    const val SHARE_AUTHORITY_SUFFIX = ".grafyn.share"
    const val SHARE_SHEET_OPENED_STATUS = "share_sheet_opened"
    const val HEALTH_READY = "ready"
    const val HEALTH_DEGRADED = "degraded"
    const val HEALTH_FATAL = "fatal"
    const val HEALTH_UNAVAILABLE = "unavailable"
    const val HEALTH_CORRUPT = "corrupt"
    const val MAX_SECRET_BYTES = 1024
    const val MAX_SHARE_FILENAME_BYTES = 128
    const val MAX_SHARE_IMAGE_BYTES = 24 * 1024 * 1024
    const val GCM_NONCE_BYTES = 12
    const val GCM_TAG_BYTES = 16

    private const val AAD_SCHEMA = "grafyn-secret-aad-v1"
    private const val DEVICE_ACCOUNT = "sync.device.ed25519.v1"
    private const val OPENROUTER_PREFIX = "openrouter_api_key/"
    private const val VAULT_PREFIX = "sync.vault."
    private const val VAULT_SUFFIX = ".root.v1"
    private val canonicalUuid =
        Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
    private val generatedShareFile = Regex(
        "grafyn-[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\\.(png|jpg|webp)",
    )
    private val encryptedSecretPart = Regex("([0-9a-f]{64})\\.(account|nonce|ciphertext|tag)")

    fun secureSecretHealth(hasDurableRecords: Boolean, keystoreReady: Boolean): String = when {
        keystoreReady -> HEALTH_READY
        hasDurableRecords -> HEALTH_CORRUPT
        else -> HEALTH_UNAVAILABLE
    }

    fun overallHealth(secureSecrets: String, nativeImageShare: String): String? {
        if (nativeImageShare != HEALTH_READY && nativeImageShare != HEALTH_UNAVAILABLE) return null
        return when (secureSecrets) {
            HEALTH_CORRUPT -> HEALTH_FATAL
            HEALTH_UNAVAILABLE -> HEALTH_DEGRADED
            HEALTH_READY -> if (nativeImageShare == HEALTH_READY) HEALTH_READY else HEALTH_DEGRADED
            else -> null
        }
    }

    fun hasCompleteSecretRecords(keys: Set<String>): Boolean {
        if (keys.isEmpty()) return false
        val partsByPrefix = mutableMapOf<String, MutableSet<String>>()
        for (key in keys) {
            val match = encryptedSecretPart.matchEntire(key) ?: return false
            partsByPrefix.getOrPut(match.groupValues[1]) { mutableSetOf() }
                .add(match.groupValues[2])
        }
        return partsByPrefix.values.all { it == setOf("account", "nonce", "ciphertext", "tag") }
    }

    fun isValidAccount(account: String): Boolean {
        if (account == DEVICE_ACCOUNT) return true
        val uuid = when {
            account.startsWith(OPENROUTER_PREFIX) -> account.removePrefix(OPENROUTER_PREFIX)
            account.startsWith(VAULT_PREFIX) && account.endsWith(VAULT_SUFFIX) ->
                account.removePrefix(VAULT_PREFIX).removeSuffix(VAULT_SUFFIX)
            else -> return false
        }
        return canonicalUuid.matches(uuid) && uuid.any { it in '1'..'9' || it in 'a'..'f' }
    }

    fun aadFor(account: String): ByteArray =
        "$AAD_SCHEMA\u0000$SECRET_SERVICE\u0000$account".toByteArray(StandardCharsets.UTF_8)

    fun authenticatesSecret(
        account: String,
        key: SecretKey,
        nonce: ByteArray,
        ciphertext: ByteArray,
        tag: ByteArray,
    ): Boolean {
        if (
            !isValidAccount(account) ||
            nonce.size != GCM_NONCE_BYTES ||
            ciphertext.isEmpty() || ciphertext.size > MAX_SECRET_BYTES ||
            tag.size != GCM_TAG_BYTES
        ) {
            return false
        }
        val sealed = ciphertext + tag
        var plaintext: ByteArray? = null
        return try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(
                Cipher.DECRYPT_MODE,
                key,
                GCMParameterSpec(GCM_TAG_BYTES * 8, nonce),
            )
            cipher.updateAAD(aadFor(account))
            val opened = cipher.doFinal(sealed)
            plaintext = opened
            opened.isNotEmpty() && opened.size <= MAX_SECRET_BYTES
        } catch (_: Exception) {
            false
        } finally {
            plaintext?.fill(0)
            sealed.fill(0)
        }
    }

    fun isValidShare(fileName: String, mime: String): Boolean {
        if (fileName.isEmpty() || fileName.toByteArray(StandardCharsets.US_ASCII).size > MAX_SHARE_FILENAME_BYTES) {
            return false
        }
        if (!generatedShareFile.matches(fileName)) return false
        return when (mime) {
            "image/png" -> fileName.endsWith(".png")
            "image/jpeg" -> fileName.endsWith(".jpg")
            "image/webp" -> fileName.endsWith(".webp")
            else -> false
        }
    }

    fun matchesImageHeader(mime: String, header: ByteArray): Boolean = when (mime) {
        "image/png" -> header.size >= 8 && header.copyOfRange(0, 8).contentEquals(
            byteArrayOf(0x89.toByte(), 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a),
        )
        "image/jpeg" -> header.size >= 3 &&
            header[0] == 0xff.toByte() && header[1] == 0xd8.toByte() && header[2] == 0xff.toByte()
        "image/webp" -> header.size >= 12 &&
            String(header, 0, 4, StandardCharsets.US_ASCII) == "RIFF" &&
            String(header, 8, 4, StandardCharsets.US_ASCII) == "WEBP"
        else -> false
    }
}
