package com.grafyn.app

import android.app.Activity
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.core.content.FileProvider
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.FileInputStream
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

@InvokeArg
class SecretPutArgs {
    var account: String = ""
    var secretBase64: String = ""
}

@InvokeArg
class SecretAccountArgs {
    var account: String = ""
}

@InvokeArg
class ShareImageArgs {
    var fileName: String = ""
    var mime: String = ""
}

private class SecretFailure(val code: String) : Exception()

private data class EncryptedSecret(
    val account: String,
    val nonce: ByteArray,
    val ciphertext: ByteArray,
    val tag: ByteArray,
)

private class AndroidKeystoreSecretStore(context: Context) {
    private val preferences =
        context.getSharedPreferences("grafyn_encrypted_secrets_v1", Context.MODE_PRIVATE)
    private val lock = Any()

    fun health(): String = synchronized(lock) {
        val recordKeys = try {
            preferences.all.keys.toSet()
        } catch (_: Exception) {
            return@synchronized GrafynAndroidContracts.HEALTH_CORRUPT
        }
        val hasDurableRecords = recordKeys.isNotEmpty()
        try {
            if (hasDurableRecords) {
                if (!GrafynAndroidContracts.hasCompleteSecretRecords(recordKeys)) {
                    throw SecretFailure("corrupt_secret")
                }
            }
            val key = if (hasDurableRecords) loadExistingKey() else loadOrCreateKey()
            if (hasDurableRecords) validateStoredRecords(recordKeys, key)
            Cipher.getInstance("AES/GCM/NoPadding")
            if (key.encoded != null) throw SecretFailure("backend_unavailable")
            GrafynAndroidContracts.secureSecretHealth(hasDurableRecords, keystoreReady = true)
        } catch (_: Exception) {
            GrafynAndroidContracts.secureSecretHealth(hasDurableRecords, keystoreReady = false)
        }
    }

    fun put(account: String, encodedSecret: String): String = synchronized(lock) {
        requireAccount(account)
        val secret = decodeCanonical(encodedSecret, "invalid_secret")
        try {
            if (secret.isEmpty()) throw SecretFailure("invalid_secret")
            if (secret.size > GrafynAndroidContracts.MAX_SECRET_BYTES) {
                throw SecretFailure("secret_too_large")
            }
            if (readRecord(account) != null) return@synchronized "already_exists"

            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, loadOrCreateKey())
            val nonce = cipher.iv
            if (nonce.size != GrafynAndroidContracts.GCM_NONCE_BYTES) {
                throw SecretFailure("backend_unavailable")
            }
            cipher.updateAAD(GrafynAndroidContracts.aadFor(account))
            val sealed = cipher.doFinal(secret)
            try {
                if (sealed.size <= GrafynAndroidContracts.GCM_TAG_BYTES) {
                    throw SecretFailure("backend_unavailable")
                }
                val split = sealed.size - GrafynAndroidContracts.GCM_TAG_BYTES
                val record = EncryptedSecret(
                    account,
                    nonce.copyOf(),
                    sealed.copyOfRange(0, split),
                    sealed.copyOfRange(split, sealed.size),
                )
                if (!writeRecord(account, record)) throw SecretFailure("backend_unavailable")

                val durable = readRecord(account) ?: throw SecretFailure("backend_unavailable")
                val readback = decrypt(durable)
                try {
                    if (!MessageDigest.isEqual(secret, readback)) {
                        throw SecretFailure("backend_unavailable")
                    }
                } finally {
                    readback.fill(0)
                }
            } finally {
                sealed.fill(0)
            }
            "stored"
        } finally {
            secret.fill(0)
        }
    }

    fun get(account: String): ByteArray? = synchronized(lock) {
        requireAccount(account)
        val record = readRecord(account) ?: return@synchronized null
        decrypt(record)
    }

    fun delete(account: String): String = synchronized(lock) {
        requireAccount(account)
        val prefix = recordPrefix(account)
        val committed = preferences.edit()
            .remove("$prefix.account")
            .remove("$prefix.nonce")
            .remove("$prefix.ciphertext")
            .remove("$prefix.tag")
            .commit()
        if (
            !committed ||
            preferences.contains("$prefix.account") ||
            recordParts(prefix).any { it != null }
        ) {
            throw SecretFailure("backend_unavailable")
        }
        "deleted"
    }

    private fun requireAccount(account: String) {
        if (!GrafynAndroidContracts.isValidAccount(account)) {
            throw SecretFailure("invalid_account")
        }
    }

    private fun loadOrCreateKey(): SecretKey {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val existing = keyStore.getKey(GrafynAndroidContracts.KEY_ALIAS, null)
        val key = when (existing) {
            null -> {
                val generator = KeyGenerator.getInstance(
                    KeyProperties.KEY_ALGORITHM_AES,
                    "AndroidKeyStore",
                )
                generator.init(
                    KeyGenParameterSpec.Builder(
                        GrafynAndroidContracts.KEY_ALIAS,
                        KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
                    )
                        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                        .setKeySize(256)
                        .setRandomizedEncryptionRequired(true)
                        .build(),
                )
                generator.generateKey()
            }
            is SecretKey -> existing
            else -> throw SecretFailure("backend_unavailable")
        }
        if (key.encoded != null) throw SecretFailure("backend_unavailable")
        return key
    }

    private fun loadExistingKey(): SecretKey {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        val key = keyStore.getKey(GrafynAndroidContracts.KEY_ALIAS, null) as? SecretKey
            ?: throw SecretFailure("corrupt_secret")
        if (key.encoded != null) throw SecretFailure("backend_unavailable")
        return key
    }

    private fun decrypt(record: EncryptedSecret): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(
            Cipher.DECRYPT_MODE,
            loadExistingKey(),
            GCMParameterSpec(GrafynAndroidContracts.GCM_TAG_BYTES * 8, record.nonce),
        )
        cipher.updateAAD(GrafynAndroidContracts.aadFor(record.account))
        val sealed = record.ciphertext + record.tag
        return try {
            val plaintext = cipher.doFinal(sealed)
            if (plaintext.isEmpty() || plaintext.size > GrafynAndroidContracts.MAX_SECRET_BYTES) {
                plaintext.fill(0)
                throw SecretFailure("corrupt_secret")
            }
            plaintext
        } catch (error: SecretFailure) {
            throw error
        } catch (_: Exception) {
            throw SecretFailure("corrupt_secret")
        } finally {
            sealed.fill(0)
        }
    }

    private fun writeRecord(account: String, record: EncryptedSecret): Boolean {
        val prefix = recordPrefix(account)
        return preferences.edit()
            .putString("$prefix.account", account)
            .putString("$prefix.nonce", encode(record.nonce))
            .putString("$prefix.ciphertext", encode(record.ciphertext))
            .putString("$prefix.tag", encode(record.tag))
            .commit()
    }

    private fun readRecord(account: String): EncryptedSecret? {
        val record = readRecordByPrefix(recordPrefix(account)) ?: return null
        if (record.account != account) {
            record.nonce.fill(0)
            record.ciphertext.fill(0)
            record.tag.fill(0)
            throw SecretFailure("corrupt_secret")
        }
        return record
    }

    private fun readRecordByPrefix(prefix: String): EncryptedSecret? {
        val account = preferences.getString("$prefix.account", null)
        val parts = recordParts(prefix)
        if (account == null && parts.all { it == null }) return null
        if (
            account == null ||
            parts.any { it == null } ||
            !GrafynAndroidContracts.isValidAccount(account) ||
            recordPrefix(account) != prefix
        ) {
            throw SecretFailure("corrupt_secret")
        }
        val nonce = decodeCanonical(parts[0]!!, "corrupt_secret")
        val ciphertext = decodeCanonical(parts[1]!!, "corrupt_secret")
        val tag = decodeCanonical(parts[2]!!, "corrupt_secret")
        if (
            nonce.size != GrafynAndroidContracts.GCM_NONCE_BYTES ||
            ciphertext.isEmpty() || ciphertext.size > GrafynAndroidContracts.MAX_SECRET_BYTES ||
            tag.size != GrafynAndroidContracts.GCM_TAG_BYTES
        ) {
            nonce.fill(0)
            ciphertext.fill(0)
            tag.fill(0)
            throw SecretFailure("corrupt_secret")
        }
        return EncryptedSecret(account, nonce, ciphertext, tag)
    }

    private fun validateStoredRecords(recordKeys: Set<String>, key: SecretKey) {
        for (prefix in recordKeys.map { it.substringBeforeLast('.') }.toSet()) {
            val record = readRecordByPrefix(prefix) ?: throw SecretFailure("corrupt_secret")
            try {
                if (!GrafynAndroidContracts.authenticatesSecret(
                        record.account,
                        key,
                        record.nonce,
                        record.ciphertext,
                        record.tag,
                    )
                ) {
                    throw SecretFailure("corrupt_secret")
                }
            } finally {
                record.nonce.fill(0)
                record.ciphertext.fill(0)
                record.tag.fill(0)
            }
        }
    }

    private fun recordParts(prefix: String): List<String?> = listOf(
        preferences.getString("$prefix.nonce", null),
        preferences.getString("$prefix.ciphertext", null),
        preferences.getString("$prefix.tag", null),
    )

    private fun recordPrefix(account: String): String =
        MessageDigest.getInstance("SHA-256")
            .digest(account.toByteArray(Charsets.UTF_8))
            .joinToString(separator = "") { byte -> "%02x".format(byte.toInt() and 0xff) }

    private fun decodeCanonical(value: String, errorCode: String): ByteArray {
        if (value.isEmpty() || value.length > 4 * ((GrafynAndroidContracts.MAX_SECRET_BYTES + 2) / 3)) {
            throw SecretFailure(errorCode)
        }
        val decoded = try {
            Base64.decode(value, Base64.DEFAULT)
        } catch (_: IllegalArgumentException) {
            throw SecretFailure(errorCode)
        }
        if (encode(decoded) != value) {
            decoded.fill(0)
            throw SecretFailure(errorCode)
        }
        return decoded
    }

    private fun encode(value: ByteArray): String = Base64.encodeToString(value, Base64.NO_WRAP)
}

@TauriPlugin
class GrafynAndroidPlugin(private val activity: Activity) : Plugin(activity) {
    private val secrets by lazy { AndroidKeystoreSecretStore(activity.applicationContext) }

    @Command
    fun health(invoke: Invoke) {
        val secureSecrets = try {
            secrets.health()
        } catch (_: Exception) {
            GrafynAndroidContracts.HEALTH_CORRUPT
        }
        val nativeImageShare = try {
            if (isShareReady()) {
                GrafynAndroidContracts.HEALTH_READY
            } else {
                GrafynAndroidContracts.HEALTH_UNAVAILABLE
            }
        } catch (_: Exception) {
            GrafynAndroidContracts.HEALTH_UNAVAILABLE
        }
        invoke.resolve(JSObject().apply {
            put(
                "status",
                GrafynAndroidContracts.overallHealth(secureSecrets, nativeImageShare)
                    ?: GrafynAndroidContracts.HEALTH_FATAL,
            )
            put("secureSecrets", secureSecrets)
            put("nativeImageShare", nativeImageShare)
        })
    }

    @Command
    fun putSecret(invoke: Invoke) {
        val status = try {
            val args = invoke.parseArgs(SecretPutArgs::class.java)
            secrets.put(args.account, args.secretBase64)
        } catch (error: SecretFailure) {
            error.code
        } catch (_: Exception) {
            "backend_unavailable"
        }
        invoke.resolve(statusReply(status))
    }

    @Command
    fun getSecret(invoke: Invoke) {
        val response = JSObject()
        var plaintext: ByteArray? = null
        try {
            val args = invoke.parseArgs(SecretAccountArgs::class.java)
            plaintext = secrets.get(args.account)
            if (plaintext == null) {
                response.put("status", "missing")
            } else {
                response.put("status", "found")
                response.put("secretBase64", Base64.encodeToString(plaintext, Base64.NO_WRAP))
            }
        } catch (error: SecretFailure) {
            response.put("status", error.code)
        } catch (_: Exception) {
            response.put("status", "backend_unavailable")
        } finally {
            plaintext?.fill(0)
        }
        invoke.resolve(response)
    }

    @Command
    fun deleteSecret(invoke: Invoke) {
        val status = try {
            val args = invoke.parseArgs(SecretAccountArgs::class.java)
            secrets.delete(args.account)
        } catch (error: SecretFailure) {
            error.code
        } catch (_: Exception) {
            "backend_unavailable"
        }
        invoke.resolve(statusReply(status))
    }

    @Command
    fun shareImage(invoke: Invoke) {
        val status = try {
            val args = invoke.parseArgs(ShareImageArgs::class.java)
            shareImage(args)
            GrafynAndroidContracts.SHARE_SHEET_OPENED_STATUS
        } catch (error: SecretFailure) {
            error.code
        } catch (_: Exception) {
            "backend_unavailable"
        }
        invoke.resolve(statusReply(status))
    }

    private fun shareImage(args: ShareImageArgs) {
        if (!GrafynAndroidContracts.isValidShare(args.fileName, args.mime)) {
            throw SecretFailure(
                if (args.mime in setOf("image/png", "image/jpeg", "image/webp")) {
                    "invalid_filename"
                } else {
                    "invalid_mime"
                },
            )
        }

        val root = File(activity.cacheDir, GrafynAndroidContracts.SHARE_DIRECTORY)
        if (!root.isDirectory || root.canonicalFile != root.absoluteFile) {
            throw SecretFailure("file_unavailable")
        }
        val requested = File(root, args.fileName).absoluteFile
        val file = requested.canonicalFile
        if (file != requested || file.parentFile != root || !file.isFile) {
            throw SecretFailure("file_unavailable")
        }
        if (file.length() <= 0 || file.length() > GrafynAndroidContracts.MAX_SHARE_IMAGE_BYTES) {
            throw SecretFailure("file_unavailable")
        }
        val header = FileInputStream(file).use { stream ->
            val buffer = ByteArray(12)
            val read = stream.read(buffer)
            if (read <= 0) ByteArray(0) else buffer.copyOf(read)
        }
        if (!GrafynAndroidContracts.matchesImageHeader(args.mime, header)) {
            throw SecretFailure("file_unavailable")
        }

        val authority = activity.packageName + GrafynAndroidContracts.SHARE_AUTHORITY_SUFFIX
        val uri = FileProvider.getUriForFile(activity, authority, file)
        val intent = Intent(Intent.ACTION_SEND).apply {
            type = args.mime
            putExtra(Intent.EXTRA_STREAM, uri)
            clipData = ClipData.newUri(activity.contentResolver, "Grafyn image", uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        activity.startActivity(Intent.createChooser(intent, null))
    }

    private fun isShareReady(): Boolean {
        val authority = activity.packageName + GrafynAndroidContracts.SHARE_AUTHORITY_SUFFIX
        val provider = activity.packageManager.resolveContentProvider(
            authority,
            PackageManager.GET_META_DATA,
        ) ?: return false
        val root = File(activity.cacheDir, GrafynAndroidContracts.SHARE_DIRECTORY)
        return !provider.exported &&
            provider.grantUriPermissions &&
            provider.authority == authority &&
            (provider.metaData?.getInt("android.support.FILE_PROVIDER_PATHS", 0) ?: 0) != 0 &&
            root.isDirectory &&
            root.canonicalFile == root.absoluteFile
    }

    private fun statusReply(status: String): JSObject = JSObject().apply {
        put("status", status)
    }
}
