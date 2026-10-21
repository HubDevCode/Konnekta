package org.konnekta.app.approve

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.Signature
import java.security.spec.ECGenParameterSpec

/**
 * One P-256 key per paired computer in the Android Keystore. Each signature
 * needs a new strong biometric check (no time window), and a new fingerprint
 * in the phone settings invalidates the key. The private key never leaves
 * the secure hardware (StrongBox when the phone has it).
 *
 * Re-enrollment keeps the current key until the new one has reached the
 * computer: keys alternate between two aliases, and the current one is
 * recorded only after a successful send.
 */
class ApproveKeys(context: Context) {
    private val prefs = context.getSharedPreferences("approve-keys", Context.MODE_PRIVATE)
    private val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    private fun alias(computerId: String, slot: String) = "konnekta-approve-$computerId.$slot"

    private fun currentSlot(computerId: String): String? = prefs.getString(computerId, null)

    fun hasKey(computerId: String): Boolean {
        val slot = currentSlot(computerId) ?: return false
        return ks.containsAlias(alias(computerId, slot))
    }

    /** Signature object for the current key, to hand to BiometricPrompt. */
    fun signerForApproval(computerId: String): Signature {
        val slot = currentSlot(computerId) ?: throw IllegalStateException("no key for this computer")
        return signer(alias(computerId, slot))
    }

    private fun signer(alias: String): Signature {
        val key = ks.getKey(alias, null) as? PrivateKey ?: throw IllegalStateException("key missing")
        return try {
            Signature.getInstance("SHA256withECDSA").apply { initSign(key) }
        } catch (e: KeyPermanentlyInvalidatedException) {
            // A new fingerprint was added: the key can never sign again.
            ks.deleteEntry(alias)
            throw IllegalStateException("the fingerprints changed: register the phone again")
        }
    }

    /** A new key in the free slot. Returns (slot, signer, SubjectPublicKeyInfo DER). */
    fun newKey(computerId: String): Triple<String, Signature, ByteArray> {
        val slot = if (currentSlot(computerId) == "a") "b" else "a"
        val alias = alias(computerId, slot)
        if (ks.containsAlias(alias)) ks.deleteEntry(alias)
        fun spec(strongBox: Boolean) = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_SIGN)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
            .setUserAuthenticationRequired(true)
            .setInvalidatedByBiometricEnrollment(true)
            .apply {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
                } else {
                    @Suppress("DEPRECATION")
                    setUserAuthenticationValidityDurationSeconds(-1)
                }
                if (strongBox) setIsStrongBoxBacked(true)
            }
            .build()
        val gen = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
        val pair = try {
            gen.initialize(spec(strongBox = true))
            gen.generateKeyPair()
        } catch (e: StrongBoxUnavailableException) {
            gen.initialize(spec(strongBox = false))
            gen.generateKeyPair()
        }
        return Triple(slot, signer(alias), pair.public.encoded)
    }

    /** The new key reached the computer: make it current, delete the old one. */
    fun commit(computerId: String, slot: String) {
        val old = currentSlot(computerId)
        prefs.edit().putString(computerId, slot).apply()
        if (old != null && old != slot) ks.deleteEntry(alias(computerId, old))
    }

    /** Enrollment failed or was cancelled: drop the new key, keep the old one. */
    fun discard(computerId: String, slot: String) {
        if (currentSlot(computerId) != slot) ks.deleteEntry(alias(computerId, slot))
    }

    /** Unpair: forget every key of the computer. */
    fun remove(computerId: String) {
        for (slot in listOf("a", "b")) ks.deleteEntry(alias(computerId, slot))
        prefs.edit().remove(computerId).apply()
    }

    companion object {
        fun sha256Hex(b: ByteArray): String =
            MessageDigest.getInstance("SHA-256").digest(b).joinToString("") { "%02x".format(it) }

        /** Code the user types on the computer: first 8 bytes of the SHA-256, in groups of 4. */
        fun code(spki: ByteArray): String =
            sha256Hex(spki).substring(0, 16).uppercase().chunked(4).joinToString(" ")
    }
}
