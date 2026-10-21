package org.konnekta.app.approve

/**
 * The exact bytes the phone signs. Must match crates/konnekta-approve/src/message.rs
 * byte for byte (the unit test uses the same vector as the Rust test).
 */
object ApproveMessage {
    const val APPROVE_V1 = "konnekta-approve-v1"
    const val ENROLL_V1 = "konnekta-approve-enroll-v1"
    const val MAX_VALUE = 256
    const val MAX_TIME = 1L shl 40

    class InvalidField(message: String) : Exception(message)

    /** Control characters, including the bidi controls that reorder text. */
    fun isControl(c: Int): Boolean =
        Character.isISOControl(c) || c == 0x061C || c == 0x200E || c == 0x200F ||
            c in 0x202A..0x202E || c in 0x2066..0x2069

    private fun check(name: String, v: String, required: Boolean) {
        if (required && v.isEmpty()) throw InvalidField("$name is empty")
        if (v.toByteArray(Charsets.UTF_8).size > MAX_VALUE) throw InvalidField("$name is too long")
        if (v.codePoints().anyMatch { isControl(it) }) throw InvalidField("$name has a control character")
    }

    private fun hex64(s: String) = s.length == 64 && s.all { it in '0'..'9' || it in 'a'..'f' }

    private fun checkTime(t: Long) {
        if (t < 1 || t > MAX_TIME) throw InvalidField("time out of range")
    }

    fun approval(host: String, user: String, service: String, tty: String, rhost: String, time: Long, nonce: String): ByteArray {
        check("host", host, true)
        check("user", user, true)
        check("service", service, true)
        check("tty", tty, false)
        check("rhost", rhost, false)
        checkTime(time)
        if (!hex64(nonce)) throw InvalidField("nonce must be 64 lowercase hex digits")
        return "$APPROVE_V1\nhost=$host\nuser=$user\nservice=$service\ntty=$tty\nrhost=$rhost\ntime=$time\nnonce=$nonce\n"
            .toByteArray(Charsets.UTF_8)
    }

    fun enrollment(host: String, user: String, keySha256: String, time: Long, nonce: String): ByteArray {
        check("host", host, true)
        check("user", user, true)
        checkTime(time)
        if (!hex64(keySha256)) throw InvalidField("key must be 64 lowercase hex digits")
        if (!hex64(nonce)) throw InvalidField("nonce must be 64 lowercase hex digits")
        return "$ENROLL_V1\nhost=$host\nuser=$user\nkey=$keySha256\ntime=$time\nnonce=$nonce\n"
            .toByteArray(Charsets.UTF_8)
    }
}
