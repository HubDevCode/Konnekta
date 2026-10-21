package org.konnekta.app.service

/** Just enough H.264 parsing to size a decoder from the stream's SPS. */
object H264 {
    private class Bits(private val b: ByteArray) {
        private var pos = 0
        fun bit(): Int {
            val v = (b[pos / 8].toInt() shr (7 - pos % 8)) and 1
            pos++
            return v
        }
        fun bits(n: Int): Int {
            var v = 0
            repeat(n) { v = (v shl 1) or bit() }
            return v
        }
        fun ue(): Int {
            var zeros = 0
            while (bit() == 0) if (++zeros > 31) error("bad ue")
            return ((1 shl zeros) - 1) + bits(zeros)
        }
        fun se(): Int = ue().let { if (it and 1 == 1) (it + 1) / 2 else -(it / 2) }
    }

    /** Drop emulation-prevention bytes (00 00 03 -> 00 00). */
    private fun rbsp(nal: ByteArray, from: Int): ByteArray {
        val out = java.io.ByteArrayOutputStream(nal.size)
        var zeros = 0
        for (i in from until nal.size) {
            val v = nal[i].toInt() and 0xff
            if (zeros >= 2 && v == 3) {
                zeros = 0
                continue
            }
            out.write(v)
            zeros = if (v == 0) zeros + 1 else 0
        }
        return out.toByteArray()
    }

    /** Width and height of an SPS NAL (with start code of [hdr] bytes), or null. */
    fun spsSize(nal: ByteArray, hdr: Int): Pair<Int, Int>? = runCatching {
        val r = Bits(rbsp(nal, hdr + 1))
        val profile = r.bits(8)
        r.bits(16) // constraints, level
        r.ue() // sps id
        var chroma = 1
        if (profile in setOf(100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135)) {
            chroma = r.ue()
            if (chroma == 3) r.bit()
            r.ue(); r.ue(); r.bit()
            if (r.bit() == 1) {
                repeat(if (chroma != 3) 8 else 12) { i ->
                    if (r.bit() == 1) {
                        var last = 8
                        var next = 8
                        repeat(if (i < 6) 16 else 64) {
                            if (next != 0) next = (last + r.se() + 256) % 256
                            if (next != 0) last = next
                        }
                    }
                }
            }
        }
        r.ue() // log2_max_frame_num
        when (r.ue()) {
            0 -> r.ue()
            1 -> {
                r.bit(); r.se(); r.se()
                repeat(r.ue()) { r.se() }
            }
        }
        r.ue(); r.bit()
        val wMbs = r.ue() + 1
        val hMap = r.ue() + 1
        val frameMbsOnly = r.bit()
        if (frameMbsOnly == 0) r.bit()
        r.bit()
        var (cl, cr, ct, cb) = listOf(0, 0, 0, 0)
        if (r.bit() == 1) {
            cl = r.ue(); cr = r.ue(); ct = r.ue(); cb = r.ue()
        }
        val cx = if (chroma == 0 || chroma == 3) 1 else 2
        val cy = (if (chroma == 1) 2 else 1) * (2 - frameMbsOnly)
        (wMbs * 16 - (cl + cr) * cx) to ((2 - frameMbsOnly) * hMap * 16 - (ct + cb) * cy)
    }.getOrNull()
}
