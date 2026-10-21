package org.konnekta.app.service

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioManager
import android.net.Uri
import android.content.BroadcastReceiver
import android.content.Intent
import android.content.IntentFilter
import android.provider.ContactsContract
import android.telecom.TelecomManager
import android.telephony.TelephonyManager
import android.util.Log
import org.json.JSONObject
import org.konnekta.app.KonnektaApp
import org.konnekta.app.core.Types
import java.util.concurrent.Executors

/**
 * Calls for the computers: `konnekta.call` with ringing / talking /
 * missedCall and a cancel when the call ends; the computer can silence the
 * ringer and, with the Answer calls permission, answer or reject.
 */
class Calls(private val ctx: Context) {
    companion object {
        const val TAG = "Konnekta"
        val PERMISSIONS = arrayOf(
            Manifest.permission.READ_PHONE_STATE,
            Manifest.permission.READ_CALL_LOG,
            Manifest.permission.READ_CONTACTS,
            Manifest.permission.ANSWER_PHONE_CALLS,
        )
    }

    private val io = Executors.newSingleThreadExecutor()
    private val audio = ctx.getSystemService(AudioManager::class.java)
    private var state = TelephonyManager.CALL_STATE_IDLE
    private var number: String? = null
    private var answered = false
    private var ringerMuted = false

    private fun granted(p: String) = ctx.checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    /**
     * The phone-state broadcast carries the caller's number (with
     * READ_CALL_LOG); the newer callbacks do not. It may arrive twice per
     * change, once without the number.
     */
    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            @Suppress("DEPRECATION")
            val n = i.getStringExtra(TelephonyManager.EXTRA_INCOMING_NUMBER)
            val s = when (i.getStringExtra(TelephonyManager.EXTRA_STATE)) {
                TelephonyManager.EXTRA_STATE_RINGING -> TelephonyManager.CALL_STATE_RINGING
                TelephonyManager.EXTRA_STATE_OFFHOOK -> TelephonyManager.CALL_STATE_OFFHOOK
                else -> TelephonyManager.CALL_STATE_IDLE
            }
            changed(s, n)
        }
    }
    private var registered = false

    fun start() {
        if (!granted(Manifest.permission.READ_PHONE_STATE) || registered) return
        ctx.registerReceiver(receiver, IntentFilter(TelephonyManager.ACTION_PHONE_STATE_CHANGED))
        registered = true
    }

    fun stop() {
        if (registered) runCatching { ctx.unregisterReceiver(receiver) }
        registered = false
        io.shutdown()
    }

    private fun changed(s: Int, phoneNumber: String?) {
        val newNumber = !phoneNumber.isNullOrEmpty() && phoneNumber != number
        if (!phoneNumber.isNullOrEmpty()) number = phoneNumber
        val prev = state
        if (s == prev && !(s == TelephonyManager.CALL_STATE_RINGING && newNumber)) return
        state = s
        when (s) {
            TelephonyManager.CALL_STATE_RINGING -> {
                if (prev != s) answered = false
                emit("ringing")
            }
            TelephonyManager.CALL_STATE_OFFHOOK -> {
                answered = true
                restoreRinger()
                emit("talking")
            }
            TelephonyManager.CALL_STATE_IDLE -> {
                restoreRinger()
                if (prev == TelephonyManager.CALL_STATE_RINGING && !answered) emit("missedCall")
                emit(if (answered) "talking" else "ringing", cancel = true)
                number = null
            }
        }
    }

    private fun contactName(n: String?): String? {
        if (n.isNullOrEmpty() || !granted(Manifest.permission.READ_CONTACTS)) return null
        return runCatching {
            ctx.contentResolver.query(
                Uri.withAppendedPath(ContactsContract.PhoneLookup.CONTENT_FILTER_URI, Uri.encode(n)),
                arrayOf(ContactsContract.PhoneLookup.DISPLAY_NAME), null, null, null,
            )?.use { c -> if (c.moveToFirst()) c.getString(0) else null }
        }.getOrNull()
    }

    private fun emit(event: String, cancel: Boolean = false) {
        val n = number
        io.execute {
            val body = JSONObject().put("event", event).put("isCancel", cancel)
            n?.let { body.put("phoneNumber", it) }
            contactName(n)?.let { body.put("contactName", it) }
            for (d in connectedPaired()) runCatching { KonnektaApp.core?.sendPacket(d, Types.CALL, body) }
        }
    }

    /** konnekta.call.mute: silence the ringer for this call only. */
    fun muteRinger() {
        if (state != TelephonyManager.CALL_STATE_RINGING || ringerMuted) return
        runCatching {
            audio.adjustStreamVolume(AudioManager.STREAM_RING, AudioManager.ADJUST_MUTE, 0)
            ringerMuted = true
        }.onFailure { Log.w(TAG, "cannot mute ringer", it) }
    }

    private fun restoreRinger() {
        if (!ringerMuted) return
        runCatching { audio.adjustStreamVolume(AudioManager.STREAM_RING, AudioManager.ADJUST_UNMUTE, 0) }
        ringerMuted = false
    }

    /** konnekta.call.action: accept / reject / hangup / mute / unmute / speaker. */
    @SuppressLint("MissingPermission")
    @Suppress("DEPRECATION")
    fun action(a: String) {
        val telecom = ctx.getSystemService(TelecomManager::class.java)
        val canAnswer = granted(Manifest.permission.ANSWER_PHONE_CALLS)
        runCatching {
            when (a) {
                "accept" -> if (canAnswer) telecom.acceptRingingCall()
                "reject", "hangup" -> if (canAnswer) telecom.endCall()
                "mute" -> audio.isMicrophoneMute = true
                "unmute" -> audio.isMicrophoneMute = false
                "speaker" -> audio.isSpeakerphoneOn = !audio.isSpeakerphoneOn
            }
        }.onFailure { Log.w(TAG, "call action $a failed", it) }
    }
}
