package org.konnekta.app.approve

import android.app.KeyguardManager
import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.Signature
import java.security.spec.ECGenParameterSpec

/**
 * One P-256 key per paired computer in the Android Keystore. The private key
 * never leaves the secure hardware (StrongBox when the phone has it). How a
 * signature is unlocked is chosen when the phone is registered ([Mode]):
 * with a strong fingerprint each signature needs a new biometric check (no
 * time window) and a new fingerprint invalidates the key; phones without one
 * (old phones, no sensor, none enrolled) approve from the request screen.
 *
 * Re-enrollment keeps the current key until the new one has reached the
 * computer: keys alternate between two aliases, and the current one is
 * recorded only after a successful send.
 */
class ApproveKeys(context: Context) {
    private val prefs = context.getSharedPreferences("approve-keys", Context.MODE_PRIVATE)
    private val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    /**
     * BIOMETRIC: strong fingerprint for every signature. UNLOCK: no fingerprint, but a secure lock
     * screen: the phone must have been unlocked (or the PIN confirmed) in the last 30 seconds.
     * TAP: no fingerprint and no lock screen: the Approve button alone signs.
     */
    enum class Mode { BIOMETRIC, UNLOCK, TAP }

    private fun alias(computerId: String, slot: String) = "konnekta-approve-$computerId.$slot"

    /** Keys made before the modes existed were all biometric. */
    fun mode(computerId: String, slot: String? = currentSlot(computerId)): Mode =
        prefs.getString("mode:$computerId.$slot", null)?.let { runCatching { Mode.valueOf(it) }.getOrNull() } ?: Mode.BIOMETRIC

    private fun currentSlot(computerId: String): String? = prefs.getString(computerId, null)

    fun hasKey(computerId: String): Boolean {
        val slot = currentSlot(computerId) ?: return false
        return ks.containsAlias(alias(computerId, slot))
    }

    fun currentSlotOrThrow(computerId: String): String =
        currentSlot(computerId) ?: throw IllegalStateException("no key for this computer")

    /**
     * Signature object for the key in `slot`: for BIOMETRIC hand it to BiometricPrompt; for UNLOCK it
     * throws UserNotAuthenticatedException when the phone was not unlocked recently.
     */
    fun signer(computerId: String, slot: String): Signature = signer(alias(computerId, slot))

    private fun signer(alias: String): Signature {
        val key = ks.getKey(alias, null) as? PrivateKey ?: throw IllegalStateException("key missing")
        return try {
            Signature.getInstance("SHA256withECDSA").apply { initSign(key) }
        } catch (e: KeyPermanentlyInvalidatedException) {
            // A new fingerprint was added, or the lock screen was removed: the key can never sign again.
            ks.deleteEntry(alias)
            throw IllegalStateException("the phone's fingerprints or lock screen changed: register the phone again")
        }
    }

    /** A new key in the free slot. Returns (slot, SubjectPublicKeyInfo DER). */
    fun newKey(computerId: String, mode: Mode): Pair<String, ByteArray> {
        val slot = if (currentSlot(computerId) == "a") "b" else "a"
        val alias = alias(computerId, slot)
        if (ks.containsAlias(alias)) ks.deleteEntry(alias)
        fun spec(strongBox: Boolean) = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_SIGN)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
            .apply {
                when (mode) {
                    Mode.BIOMETRIC -> {
                        setUserAuthenticationRequired(true)
                        // Android 6 always invalidates the key when a fingerprint is added.
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) setInvalidatedByBiometricEnrollment(true)
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                            setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
                        } else {
                            @Suppress("DEPRECATION")
                            setUserAuthenticationValidityDurationSeconds(-1)
                        }
                    }
                    Mode.UNLOCK -> {
                        setUserAuthenticationRequired(true)
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                            setUserAuthenticationParameters(UNLOCK_SECONDS, KeyProperties.AUTH_DEVICE_CREDENTIAL or KeyProperties.AUTH_BIOMETRIC_STRONG)
                        } else {
                            @Suppress("DEPRECATION")
                            setUserAuthenticationValidityDurationSeconds(UNLOCK_SECONDS)
                        }
                    }
                    Mode.TAP -> {}
                }
                if (strongBox && Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) setIsStrongBoxBacked(true)
            }
            .build()
        val gen = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
        val pair = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            try {
                gen.initialize(spec(strongBox = true))
                gen.generateKeyPair()
            } catch (e: StrongBoxUnavailableException) {
                gen.initialize(spec(strongBox = false))
                gen.generateKeyPair()
            }
        } else {
            gen.initialize(spec(strongBox = false))
            gen.generateKeyPair()
        }
        prefs.edit().putString("mode:$computerId.$slot", mode.name).apply()
        return slot to pair.public.encoded
    }

    /** The new key reached the computer: make it current, delete the old one. */
    fun commit(computerId: String, slot: String) {
        val old = currentSlot(computerId)
        prefs.edit().putString(computerId, slot).apply()
        if (old != null && old != slot) forget(computerId, old)
    }

    private fun forget(computerId: String, slot: String) {
        ks.deleteEntry(alias(computerId, slot))
        prefs.edit().remove("mode:$computerId.$slot").apply()
    }

    /** Enrollment failed or was cancelled: drop the new key, keep the old one. */
    fun discard(computerId: String, slot: String) {
        if (currentSlot(computerId) != slot) forget(computerId, slot)
    }

    /** Unpair: forget every key of the computer. */
    fun remove(computerId: String) {
        for (slot in listOf("a", "b")) forget(computerId, slot)
        prefs.edit().remove(computerId).apply()
    }

    companion object {
        /** UNLOCK keys sign for this long after the phone was unlocked or the PIN confirmed. */
        const val UNLOCK_SECONDS = 30

        /** How a new key will be unlocked on this phone, as it is now. */
        fun modeFor(context: Context): Mode = when {
            BiometricManager.from(context).canAuthenticate(BIOMETRIC_STRONG) == BiometricManager.BIOMETRIC_SUCCESS -> Mode.BIOMETRIC
            context.getSystemService(KeyguardManager::class.java).isDeviceSecure -> Mode.UNLOCK
            else -> Mode.TAP
        }

        fun sha256Hex(b: ByteArray): String =
            MessageDigest.getInstance("SHA-256").digest(b).joinToString("") { "%02x".format(it) }

        /** Code the user types on the computer: first 8 bytes of the SHA-256, in groups of 4. */
        fun code(spki: ByteArray): String =
            sha256Hex(spki).substring(0, 16).uppercase().chunked(4).joinToString(" ")
    }
}
