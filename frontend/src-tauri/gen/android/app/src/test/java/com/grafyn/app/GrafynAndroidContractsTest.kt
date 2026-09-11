package com.grafyn.app

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

class GrafynAndroidContractsTest {
    private data class SealedSecret(
        val account: String,
        val key: SecretKey,
        val nonce: ByteArray,
        val ciphertext: ByteArray,
        val tag: ByteArray,
    )

    @Test fun keystoreFailureIsFatalOnlyWhenDurableRecordsExist() {
        assertEquals(
            GrafynAndroidContracts.HEALTH_READY,
            GrafynAndroidContracts.secureSecretHealth(
                hasDurableRecords = false,
                keystoreReady = true,
            ),
        )
        assertEquals(
            GrafynAndroidContracts.HEALTH_UNAVAILABLE,
            GrafynAndroidContracts.secureSecretHealth(
                hasDurableRecords = false,
                keystoreReady = false,
            ),
        )
        assertEquals(
            GrafynAndroidContracts.HEALTH_CORRUPT,
            GrafynAndroidContracts.secureSecretHealth(
                hasDurableRecords = true,
                keystoreReady = false,
            ),
        )
    }

    @Test fun overallHealthUsesOnlyFixedConsistentStates() {
        assertEquals(
            GrafynAndroidContracts.HEALTH_READY,
            GrafynAndroidContracts.overallHealth("ready", "ready"),
        )
        assertEquals(
            GrafynAndroidContracts.HEALTH_DEGRADED,
            GrafynAndroidContracts.overallHealth("unavailable", "ready"),
        )
        assertEquals(
            GrafynAndroidContracts.HEALTH_FATAL,
            GrafynAndroidContracts.overallHealth("corrupt", "unavailable"),
        )
        assertEquals(null, GrafynAndroidContracts.overallHealth("unknown", "ready"))
        assertEquals(null, GrafynAndroidContracts.overallHealth("ready", "unknown"))
    }

    @Test fun durableSecretRecordsRequireOneCompleteCanonicalPartSet() {
        val prefix = "a".repeat(64)
        assertTrue(GrafynAndroidContracts.hasCompleteSecretRecords(setOf(
            "$prefix.account",
            "$prefix.nonce",
            "$prefix.ciphertext",
            "$prefix.tag",
        )))
        assertFalse(GrafynAndroidContracts.hasCompleteSecretRecords(setOf(
            "$prefix.account",
            "$prefix.nonce",
            "$prefix.ciphertext",
        )))
        assertFalse(GrafynAndroidContracts.hasCompleteSecretRecords(setOf(
            "$prefix.account",
            "$prefix.nonce",
            "$prefix.ciphertext",
            "$prefix.tag",
            "unexpected",
        )))
    }

    @Test fun chooserLaunchStatusDoesNotClaimRecipientDelivery() {
        assertEquals(
            "share_sheet_opened",
            GrafynAndroidContracts.SHARE_SHEET_OPENED_STATUS,
        )
    }

    @Test fun aadBindsSchemaServiceAndExactAccount() {
        assertArrayEquals(
            "grafyn-secret-aad-v1\u0000com.grafyn.app\u0000sync.device.ed25519.v1".toByteArray(),
            GrafynAndroidContracts.aadFor("sync.device.ed25519.v1"),
        )
    }

    @Test fun sameLengthCiphertextTamperingIsCorruptAndFatalSafe() {
        val sealed = sealSecret("sync.device.ed25519.v1")
        val tamperedCiphertext = sealed.ciphertext.copyOf().also {
            it[0] = (it[0].toInt() xor 1).toByte()
        }

        val health = authenticatedHealth(sealed.copy(ciphertext = tamperedCiphertext))

        assertEquals(GrafynAndroidContracts.HEALTH_CORRUPT, health)
        assertEquals(
            GrafynAndroidContracts.HEALTH_FATAL,
            GrafynAndroidContracts.overallHealth(
                health,
                GrafynAndroidContracts.HEALTH_READY,
            ),
        )
    }

    @Test fun sameLengthTagTamperingIsCorruptAndFatalSafe() {
        val sealed = sealSecret("sync.device.ed25519.v1")
        val tamperedTag = sealed.tag.copyOf().also {
            it[it.lastIndex] = (it.last().toInt() xor 1).toByte()
        }

        val health = authenticatedHealth(sealed.copy(tag = tamperedTag))

        assertEquals(GrafynAndroidContracts.HEALTH_CORRUPT, health)
        assertEquals(
            GrafynAndroidContracts.HEALTH_FATAL,
            GrafynAndroidContracts.overallHealth(
                health,
                GrafynAndroidContracts.HEALTH_READY,
            ),
        )
    }

    @Test fun intactRecordAuthenticatesWithItsExactAccountAad() {
        val sealed = sealSecret("sync.device.ed25519.v1")

        assertEquals(GrafynAndroidContracts.HEALTH_READY, authenticatedHealth(sealed))
        assertFalse(
            GrafynAndroidContracts.authenticatesSecret(
                "openrouter_api_key/123e4567-e89b-42d3-a456-426614174000",
                sealed.key,
                sealed.nonce,
                sealed.ciphertext,
                sealed.tag,
            ),
        )
    }

    @Test fun onlyTypedOpenRouterAndSyncAccountsAreAccepted() {
        assertTrue(GrafynAndroidContracts.isValidAccount("openrouter_api_key/123e4567-e89b-42d3-a456-426614174000"))
        assertTrue(GrafynAndroidContracts.isValidAccount("sync.device.ed25519.v1"))
        assertTrue(GrafynAndroidContracts.isValidAccount("sync.vault.123e4567-e89b-42d3-a456-426614174000.root.v1"))
        assertFalse(GrafynAndroidContracts.isValidAccount("openrouter_api_key/not-a-uuid"))
        assertFalse(GrafynAndroidContracts.isValidAccount("sync.vault.00000000-0000-0000-0000-000000000000.root.v1"))
        assertFalse(GrafynAndroidContracts.isValidAccount("unrelated.account"))
    }

    @Test fun shareValidationRejectsTraversalAndMimeExtensionMismatches() {
        val id = "123e4567-e89b-42d3-a456-426614174000"
        assertTrue(GrafynAndroidContracts.isValidShare("grafyn-$id.png", "image/png"))
        assertTrue(GrafynAndroidContracts.isValidShare("grafyn-$id.jpg", "image/jpeg"))
        assertTrue(GrafynAndroidContracts.isValidShare("grafyn-$id.webp", "image/webp"))
        assertFalse(GrafynAndroidContracts.isValidShare("../grafyn-$id.png", "image/png"))
        assertFalse(GrafynAndroidContracts.isValidShare("grafyn/$id.png", "image/png"))
        assertFalse(GrafynAndroidContracts.isValidShare("grafyn-$id.jpg", "image/png"))
        assertFalse(GrafynAndroidContracts.isValidShare("grafyn-$id.png", "image/jpeg"))
        assertFalse(GrafynAndroidContracts.isValidShare("grafyn-$id.jpeg", "image/jpeg"))
        assertFalse(GrafynAndroidContracts.isValidShare("grafyn-$id.gif", "image/gif"))
        assertFalse(GrafynAndroidContracts.isValidShare("grafyn-${id.uppercase()}.png", "image/png"))
        assertFalse(GrafynAndroidContracts.isValidShare("grafyn-123e4567-e89b-12d3-a456-426614174000.png", "image/png"))
    }

    @Test fun imageHeadersMustMatchTheDeclaredMime() {
        val png = byteArrayOf(0x89.toByte(), 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a)
        val jpeg = byteArrayOf(0xff.toByte(), 0xd8.toByte(), 0xff.toByte())
        val webp = "RIFF0000WEBP".toByteArray(Charsets.US_ASCII)

        assertTrue(GrafynAndroidContracts.matchesImageHeader("image/png", png))
        assertTrue(GrafynAndroidContracts.matchesImageHeader("image/jpeg", jpeg))
        assertTrue(GrafynAndroidContracts.matchesImageHeader("image/webp", webp))
        assertFalse(GrafynAndroidContracts.matchesImageHeader("image/jpeg", png))
        assertFalse(GrafynAndroidContracts.matchesImageHeader("image/png", jpeg))
        assertFalse(GrafynAndroidContracts.matchesImageHeader("image/webp", jpeg))
    }

    private fun authenticatedHealth(sealed: SealedSecret): String =
        GrafynAndroidContracts.secureSecretHealth(
            hasDurableRecords = true,
            keystoreReady = GrafynAndroidContracts.authenticatesSecret(
                sealed.account,
                sealed.key,
                sealed.nonce,
                sealed.ciphertext,
                sealed.tag,
            ),
        )

    private fun sealSecret(account: String): SealedSecret {
        val key = SecretKeySpec(ByteArray(32) { index -> (index + 1).toByte() }, "AES")
        val nonce = ByteArray(GrafynAndroidContracts.GCM_NONCE_BYTES) { index ->
            (index + 11).toByte()
        }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(
            Cipher.ENCRYPT_MODE,
            key,
            GCMParameterSpec(GrafynAndroidContracts.GCM_TAG_BYTES * 8, nonce),
        )
        cipher.updateAAD(GrafynAndroidContracts.aadFor(account))
        val sealed = cipher.doFinal("unit-test-secret".toByteArray())
        val split = sealed.size - GrafynAndroidContracts.GCM_TAG_BYTES
        return SealedSecret(
            account = account,
            key = key,
            nonce = nonce,
            ciphertext = sealed.copyOfRange(0, split),
            tag = sealed.copyOfRange(split, sealed.size),
        )
    }
}
