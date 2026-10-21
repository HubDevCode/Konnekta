package org.konnekta.app.approve

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

/** Same vectors as crates/konnekta-approve/src/message.rs. */
class ApproveMessageTest {
    private val nonce = "ab".repeat(32)

    @Test
    fun approvalBytes() {
        val m = ApproveMessage.approval("arch", "naitsirk", "sudo", "/dev/pts/3", "", 1_760_000_000, nonce)
        assertEquals(
            "konnekta-approve-v1\nhost=arch\nuser=naitsirk\nservice=sudo\ntty=/dev/pts/3\nrhost=\ntime=1760000000\nnonce=$nonce\n",
            String(m, Charsets.UTF_8),
        )
    }

    @Test
    fun enrollmentBytes() {
        val m = ApproveMessage.enrollment("arch", "u", "0".repeat(64), 5, "f".repeat(64))
        assertEquals(
            "konnekta-approve-enroll-v1\nhost=arch\nuser=u\nkey=${"0".repeat(64)}\ntime=5\nnonce=${"f".repeat(64)}\n",
            String(m, Charsets.UTF_8),
        )
    }

    @Test
    fun aValueCanNeverAddALine() {
        val bad = listOf<() -> Unit>(
            { ApproveMessage.approval("arch", "u", "sudo", "x\nservice=login", "", 5, nonce) },
            { ApproveMessage.approval("evil‮", "u", "sudo", "", "", 5, nonce) },
            { ApproveMessage.approval("arch", "u", "sudo", "", "", 5, "AB".repeat(32)) },
            { ApproveMessage.approval("arch", "u", "", "", "", 5, nonce) },
            { ApproveMessage.approval("arch", "u", "sudo", "", "", 0, nonce) },
            { ApproveMessage.approval("arch", "u".repeat(257), "sudo", "", "", 5, nonce) },
        )
        for (f in bad) assertThrows(ApproveMessage.InvalidField::class.java) { f() }
    }
}
