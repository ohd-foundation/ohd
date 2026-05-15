package com.ohd.connect.data

/**
 * Self-contained QR-code encoder — no external dependency.
 *
 * Connect bundles ML Kit's barcode *scanner* (read path) but nothing that
 * *generates* a QR. The share-detail screen needs a QR for in-person handover
 * of the share link, so this implements the QR generation directly: byte-mode
 * encoding, error-correction level M, automatic version selection (1..40),
 * Reed-Solomon ECC, the standard mask-0 pattern.
 *
 * Output is a square [QrMatrix] of booleans (true = dark module) the Compose
 * layer paints onto a Canvas. The encoder targets ASCII / UTF-8 byte content,
 * which is all a share URL ever is.
 */
object QrEncoder {

    /** A square grid of modules. `size` is the side length in modules. */
    class QrMatrix(val size: Int) {
        private val cells = BooleanArray(size * size)
        fun get(x: Int, y: Int): Boolean = cells[y * size + x]
        internal fun set(x: Int, y: Int, v: Boolean) { cells[y * size + x] = v }
    }

    /**
     * Encode [text] as a QR matrix. Throws [IllegalArgumentException] when the
     * content does not fit version 40 — a share URL never approaches that.
     */
    fun encode(text: String): QrMatrix {
        val data = text.toByteArray(Charsets.UTF_8)
        val version = pickVersion(data.size)
        val ecc = eccCodewords(version)
        val totalCodewords = totalDataCodewords(version)

        // --- bitstream: mode (byte=0100) + length + data + terminator ---
        val bits = BitBuffer()
        bits.append(0b0100, 4)
        bits.append(data.size, if (version < 10) 8 else 16)
        for (b in data) bits.append(b.toInt() and 0xFF, 8)
        // Terminator + pad to byte boundary.
        val capacityBits = totalCodewords * 8
        repeat(minOf(4, capacityBits - bits.size())) { bits.append(0, 1) }
        while (bits.size() % 8 != 0) bits.append(0, 1)
        // Pad bytes.
        val padBytes = intArrayOf(0xEC, 0x11)
        var padIdx = 0
        while (bits.size() < capacityBits) {
            bits.append(padBytes[padIdx], 8)
            padIdx = 1 - padIdx
        }

        val dataCodewords = bits.toBytes()
        val finalCodewords = interleave(dataCodewords, version, ecc)

        val size = 17 + version * 4
        val matrix = QrMatrix(size)
        val reserved = Array(size) { BooleanArray(size) }
        drawFunctionPatterns(matrix, reserved, version)
        placeData(matrix, reserved, finalCodewords)
        applyMask0(matrix, reserved)
        drawFormatInfo(matrix)
        return matrix
    }

    // ---- version + capacity tables (EC level M) ----------------------------

    // Data codewords (after ECC subtracted) per version, EC level M, v1..40.
    private val DATA_CW_M = intArrayOf(
        16, 28, 44, 64, 86, 108, 124, 154, 182, 216,
        254, 290, 334, 365, 415, 453, 507, 563, 627, 669,
        714, 782, 860, 914, 1000, 1062, 1128, 1193, 1267, 1373,
        1455, 1541, 1631, 1725, 1812, 1914, 1992, 2102, 2216, 2334,
    )
    // ECC codewords per block, EC level M.
    private val ECC_PER_BLOCK_M = intArrayOf(
        10, 16, 26, 18, 24, 16, 18, 22, 22, 26,
        30, 22, 22, 24, 24, 28, 28, 26, 26, 26,
        26, 28, 28, 28, 28, 28, 28, 28, 28, 28,
        28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    )
    // Number of EC blocks, EC level M.
    private val BLOCKS_M = intArrayOf(
        1, 1, 1, 2, 2, 4, 4, 4, 5, 5,
        5, 8, 9, 9, 10, 10, 11, 13, 14, 16,
        17, 17, 18, 20, 21, 23, 25, 26, 28, 29,
        31, 33, 35, 37, 38, 40, 43, 45, 47, 49,
    )

    private fun totalDataCodewords(version: Int): Int = DATA_CW_M[version - 1]
    private fun eccCodewords(version: Int): Int = ECC_PER_BLOCK_M[version - 1]

    private fun pickVersion(dataLen: Int): Int {
        for (v in 1..40) {
            val headerBits = 4 + (if (v < 10) 8 else 16)
            val capacityBytes = (totalDataCodewords(v) * 8 - headerBits) / 8
            if (dataLen <= capacityBytes) return v
        }
        throw IllegalArgumentException("Content too large for a QR code ($dataLen bytes)")
    }

    // ---- bit buffer --------------------------------------------------------

    private class BitBuffer {
        private val bits = ArrayList<Boolean>()
        fun append(value: Int, length: Int) {
            for (i in length - 1 downTo 0) bits.add(((value ushr i) and 1) == 1)
        }
        fun size(): Int = bits.size
        fun toBytes(): IntArray {
            val out = IntArray(bits.size / 8)
            for (i in out.indices) {
                var b = 0
                for (j in 0 until 8) if (bits[i * 8 + j]) b = b or (1 shl (7 - j))
                out[i] = b
            }
            return out
        }
    }

    // ---- Reed-Solomon over GF(256) ----------------------------------------

    private val expTable = IntArray(512)
    private val logTable = IntArray(256)

    init {
        var x = 1
        for (i in 0 until 255) {
            expTable[i] = x
            logTable[x] = i
            x = x shl 1
            if (x and 0x100 != 0) x = x xor 0x11D
        }
        for (i in 255 until 512) expTable[i] = expTable[i - 255]
    }

    private fun gfMul(a: Int, b: Int): Int =
        if (a == 0 || b == 0) 0 else expTable[logTable[a] + logTable[b]]

    private fun rsGeneratorPoly(degree: Int): IntArray {
        var poly = intArrayOf(1)
        for (i in 0 until degree) {
            val next = IntArray(poly.size + 1)
            for (j in poly.indices) {
                next[j] = next[j] xor poly[j]
                next[j + 1] = next[j + 1] xor gfMul(poly[j], expTable[i])
            }
            poly = next
        }
        return poly
    }

    private fun rsEncode(data: IntArray, eccLen: Int): IntArray {
        val gen = rsGeneratorPoly(eccLen)
        val res = IntArray(eccLen)
        for (b in data) {
            val factor = b xor res[0]
            for (i in 0 until eccLen - 1) res[i] = res[i + 1] xor gfMul(gen[i + 1], factor)
            res[eccLen - 1] = gfMul(gen[eccLen], factor)
        }
        return res
    }

    // ---- block interleaving -----------------------------------------------

    private fun interleave(data: IntArray, version: Int, eccPerBlock: Int): IntArray {
        val numBlocks = BLOCKS_M[version - 1]
        val totalData = data.size
        val shortLen = totalData / numBlocks
        val numLong = totalData % numBlocks

        val dataBlocks = ArrayList<IntArray>()
        val eccBlocks = ArrayList<IntArray>()
        var offset = 0
        for (b in 0 until numBlocks) {
            val len = shortLen + if (b >= numBlocks - numLong) 1 else 0
            val block = data.copyOfRange(offset, offset + len)
            offset += len
            dataBlocks.add(block)
            eccBlocks.add(rsEncode(block, eccPerBlock))
        }
        val out = ArrayList<Int>()
        val maxData = dataBlocks.maxOf { it.size }
        for (i in 0 until maxData) {
            for (block in dataBlocks) if (i < block.size) out.add(block[i])
        }
        for (i in 0 until eccPerBlock) {
            for (block in eccBlocks) out.add(block[i])
        }
        return out.toIntArray()
    }

    // ---- matrix layout -----------------------------------------------------

    private fun drawFunctionPatterns(m: QrMatrix, reserved: Array<BooleanArray>, version: Int) {
        val size = m.size
        fun finder(ox: Int, oy: Int) {
            for (dy in -1..7) for (dx in -1..7) {
                val x = ox + dx
                val y = oy + dy
                if (x in 0 until size && y in 0 until size) {
                    val dark = dx in 0..6 && dy in 0..6 &&
                        (dx == 0 || dx == 6 || dy == 0 || dy == 6 ||
                            (dx in 2..4 && dy in 2..4))
                    m.set(x, y, dark)
                    reserved[y][x] = true
                }
            }
        }
        finder(0, 0)
        finder(size - 7, 0)
        finder(0, size - 7)

        // Timing patterns.
        for (i in 8 until size - 8) {
            val v = i % 2 == 0
            m.set(i, 6, v); reserved[6][i] = true
            m.set(6, i, v); reserved[i][6] = true
        }
        // Dark module.
        m.set(8, size - 8, true); reserved[size - 8][8] = true

        // Alignment patterns.
        val centers = alignmentCenters(version)
        for (cy in centers) for (cx in centers) {
            if ((cx <= 8 && cy <= 8) || (cx >= size - 9 && cy <= 8) ||
                (cx <= 8 && cy >= size - 9)
            ) {
                continue
            }
            for (dy in -2..2) for (dx in -2..2) {
                val x = cx + dx
                val y = cy + dy
                val dark = dx == -2 || dx == 2 || dy == -2 || dy == 2 || (dx == 0 && dy == 0)
                m.set(x, y, dark)
                reserved[y][x] = true
            }
        }
        // Reserve format-info areas.
        for (i in 0..8) {
            if (i != 6) {
                reserved[8][i] = true
                reserved[i][8] = true
            }
        }
        for (i in 0..7) {
            reserved[8][size - 1 - i] = true
            reserved[size - 1 - i][8] = true
        }
        // Reserve version-info areas (v >= 7).
        if (version >= 7) {
            for (i in 0..5) for (j in 0..2) {
                reserved[i][size - 11 + j] = true
                reserved[size - 11 + j][i] = true
            }
        }
    }

    private fun alignmentCenters(version: Int): IntArray {
        if (version == 1) return IntArray(0)
        val count = version / 7 + 2
        val size = 17 + version * 4
        val last = size - 7
        val first = 6
        if (count == 2) return intArrayOf(first, last)
        val step = ((last - first) / (count - 1) + 1) and 0x7FFE.inv().inv() // even
        val realStep = run {
            var s = (last - first + count - 2) / (count - 1)
            if (s % 2 != 0) s += 1
            s
        }
        val centers = IntArray(count)
        centers[0] = first
        for (i in 1 until count) centers[i] = last - (count - 1 - i) * realStep
        return centers
    }

    private fun placeData(m: QrMatrix, reserved: Array<BooleanArray>, codewords: IntArray) {
        val size = m.size
        var bitIndex = 0
        val totalBits = codewords.size * 8
        var col = size - 1
        while (col > 0) {
            if (col == 6) col-- // skip vertical timing column
            for (row in 0 until size) {
                for (c in 0..1) {
                    val x = col - c
                    val upward = ((col + 1) / 2) % 2 == 0
                    val y = if (upward) size - 1 - row else row
                    if (!reserved[y][x]) {
                        val dark = if (bitIndex < totalBits) {
                            val cw = codewords[bitIndex / 8]
                            ((cw ushr (7 - bitIndex % 8)) and 1) == 1
                        } else {
                            false
                        }
                        m.set(x, y, dark)
                        bitIndex++
                    }
                }
            }
            col -= 2
        }
    }

    private fun applyMask0(m: QrMatrix, reserved: Array<BooleanArray>) {
        val size = m.size
        for (y in 0 until size) for (x in 0 until size) {
            if (!reserved[y][x] && (x + y) % 2 == 0) {
                m.set(x, y, !m.get(x, y))
            }
        }
    }

    private fun drawFormatInfo(m: QrMatrix) {
        // EC level M (0b00) + mask 0 (0b000) → format bits with BCH = 0x5412.
        val format = 0x5412
        val size = m.size
        for (i in 0..14) {
            val bit = ((format ushr i) and 1) == 1
            // Around top-left finder.
            when {
                i < 6 -> m.set(8, i, bit)
                i == 6 -> m.set(8, 7, bit)
                i == 7 -> m.set(8, 8, bit)
                i == 8 -> m.set(7, 8, bit)
                else -> m.set(14 - i, 8, bit)
            }
            // Around the other two finders.
            when {
                i < 8 -> m.set(size - 1 - i, 8, bit)
                else -> m.set(8, size - 15 + i, bit)
            }
        }
    }
}
