package com.ohd.connect.data

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.BufferedReader
import java.io.InputStream
import java.io.InputStreamReader
import java.time.Instant
import java.time.LocalDate
import java.time.LocalDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter

/**
 * Pure parser for the generic CSV import flow.
 *
 * The screen ([com.ohd.connect.ui.screens.import_.ImportCsvScreen]) drives
 * the flow:
 *   1. Open a stream over the user-picked URI, call [CsvImporter.preview] to
 *      surface the first five rows so the user can map columns.
 *   2. The user picks an `event_type`, a timestamp source, and per-column
 *      mappings; the screen calls [CsvImporter.import] which streams the
 *      file a second time, emitting one `EventInput` per data row via
 *      [StorageRepository.putEvent].
 *
 * The parser is RFC-4180-ish:
 *   - Auto-detects comma vs semicolon by sniffing the header line.
 *   - Quoted fields (`"…"`) with `""` for embedded double quotes.
 *   - LF / CRLF line endings (newlines inside quoted fields are preserved).
 *   - First non-empty line is treated as the header.
 *   - Empty lines mid-file are skipped.
 *
 * No external CSV dep — we deliberately stay on `java.io` so the importer
 * stays small and pulls no extra APK weight.
 */
data class CsvPreview(
    val headers: List<String>,
    /** Up to 5 data rows. Cells beyond the header are dropped; short rows are padded with "". */
    val firstRows: List<List<String>>,
    /** Null when we didn't bother counting — preview never counts the whole file. */
    val totalRowEstimate: Int?,
)

data class CsvColumnMapping(
    val columnIndex: Int,
    val mode: Mode,
) {
    sealed interface Mode {
        data object Skip : Mode
        data object Timestamp : Mode
        data class Channel(
            val path: String,
            val type: ChannelType,
            val unit: String?,
        ) : Mode
    }
}

object CsvImporter {

    sealed interface TimestampMode {
        data object NowForAllRows : TimestampMode
        data class FromColumn(val columnIndex: Int) : TimestampMode
    }

    /**
     * Read up to 5 data rows + the header. Returns [Result.failure] when the
     * stream is empty or the header line cannot be tokenised.
     *
     * The stream is consumed and closed by this call — callers must reopen
     * the SAF document for [import].
     */
    suspend fun preview(stream: InputStream): Result<CsvPreview> = withContext(Dispatchers.IO) {
        runCatching {
            BufferedReader(InputStreamReader(stream, Charsets.UTF_8)).use { reader ->
                val tokens = Tokeniser.open(reader) ?: error("File is empty")
                val headers = tokens.nextRow() ?: error("First line has no header columns")
                if (headers.isEmpty() || headers.all { it.isBlank() }) {
                    error("First line has no header columns")
                }
                val width = headers.size
                val firstRows = mutableListOf<List<String>>()
                while (firstRows.size < 5) {
                    val row = tokens.nextRow() ?: break
                    if (row.all { it.isBlank() }) continue
                    firstRows += normaliseRow(row, width)
                }
                CsvPreview(headers = headers, firstRows = firstRows, totalRowEstimate = null)
            }
        }
    }

    /**
     * Stream the CSV a second time and emit one event per non-empty row.
     *
     * Row-level errors (bad timestamp, bad scalar parse, storage rejection)
     * are counted as `errors` and the first message is captured in
     * `firstError`; we don't abort the import on a per-row failure.
     */
    suspend fun import(
        stream: InputStream,
        eventType: String,
        timestampMode: TimestampMode,
        mappings: List<CsvColumnMapping>,
        source: String,
    ): ImportSummary = withContext(Dispatchers.IO) {
        var emitted = 0
        var errors = 0
        var firstError: String? = null
        fun recordError(msg: String) {
            errors++
            if (firstError == null) firstError = msg
        }

        BufferedReader(InputStreamReader(stream, Charsets.UTF_8)).use { reader ->
            val tokens = Tokeniser.open(reader) ?: return@use
            val header = tokens.nextRow() ?: return@use
            val width = header.size

            var rowNumber = 1  // header is line 1; data rows start at 2
            while (true) {
                val row = tokens.nextRow() ?: break
                rowNumber++
                if (row.all { it.isBlank() }) continue
                val cells = normaliseRow(row, width)

                val tsMs = resolveTimestamp(cells, timestampMode)
                if (tsMs == null) {
                    recordError("Row $rowNumber: unparseable timestamp")
                    continue
                }

                val channels = mutableListOf<EventChannelInput>()
                var rowFailed = false
                for (m in mappings) {
                    val mode = m.mode
                    if (mode !is CsvColumnMapping.Mode.Channel) continue
                    val raw = cells.getOrNull(m.columnIndex).orEmpty().trim()
                    if (raw.isEmpty()) continue
                    val scalar = parseScalar(raw, mode.type)
                    if (scalar == null) {
                        recordError("Row $rowNumber: column '${mode.path}' value '$raw' is not ${mode.type}")
                        rowFailed = true
                        break
                    }
                    channels += EventChannelInput(path = mode.path, scalar = scalar)
                }
                if (rowFailed) continue

                val input = EventInput(
                    timestampMs = tsMs,
                    eventType = eventType,
                    channels = channels,
                    source = source,
                )
                val outcome = StorageRepository.putEvent(input).getOrElse { e ->
                    PutEventOutcome.Error(code = "INTERNAL", message = e.message ?: "putEvent failed")
                }
                when (outcome) {
                    is PutEventOutcome.Committed -> emitted++
                    is PutEventOutcome.Pending -> emitted++  // pending counts as accepted for import
                    is PutEventOutcome.Error -> recordError("Row $rowNumber: ${outcome.message}")
                }
            }
        }

        ImportSummary(emitted = emitted, errors = errors, firstError = firstError)
    }

    // -------------------------------------------------------------------------
    // Tokeniser — single-pass over a [BufferedReader].
    //
    // Sniffs the separator on construction by buffering the first non-empty
    // line, then re-tokenises that buffered line for the header row before
    // delegating to the underlying stream for subsequent rows.
    // -------------------------------------------------------------------------

    private class Tokeniser private constructor(
        private val reader: BufferedReader,
        private val sep: Char,
        private var pending: String?,
    ) {
        companion object {
            fun open(reader: BufferedReader): Tokeniser? {
                // Pull the first non-empty line for separator detection.
                while (true) {
                    val line = reader.readLine() ?: return null
                    if (line.isNotBlank()) {
                        val sep = if (line.contains(',')) ',' else ';'
                        return Tokeniser(reader, sep, line)
                    }
                }
            }
        }

        /**
         * Read one logical row. Handles quoted fields with embedded newlines
         * and `""` escapes. Returns null on EOF.
         */
        fun nextRow(): List<String>? {
            val firstLine = pending ?: reader.readLine() ?: return null
            pending = null

            // Fast path — no quotes anywhere on the line.
            if (!firstLine.contains('"')) {
                if (firstLine.isBlank()) return nextRow()  // skip empty line
                return splitNoQuotes(firstLine, sep)
            }
            return parseQuoted(firstLine)
        }

        /**
         * Parse a row that contains at least one quote. May consume
         * additional lines if a quoted field straddles a line break.
         */
        private fun parseQuoted(firstLine: String): List<String> {
            val cells = mutableListOf<String>()
            val cell = StringBuilder()
            var inQuotes = false
            var line: String? = firstLine

            outer@ while (line != null) {
                var i = 0
                while (i < line.length) {
                    val c = line[i]
                    if (inQuotes) {
                        if (c == '"') {
                            if (i + 1 < line.length && line[i + 1] == '"') {
                                cell.append('"')
                                i += 2
                                continue
                            } else {
                                inQuotes = false
                                i++
                                continue
                            }
                        } else {
                            cell.append(c)
                            i++
                        }
                    } else {
                        when {
                            c == '"' && cell.isEmpty() -> {
                                inQuotes = true
                                i++
                            }
                            c == sep -> {
                                cells += cell.toString()
                                cell.setLength(0)
                                i++
                            }
                            else -> {
                                cell.append(c)
                                i++
                            }
                        }
                    }
                }
                if (inQuotes) {
                    cell.append('\n')
                    line = reader.readLine()
                    continue@outer
                }
                cells += cell.toString()
                return cells
            }
            // EOF while still parsing — emit whatever we have.
            cells += cell.toString()
            return cells
        }
    }

    private fun splitNoQuotes(line: String, sep: Char): List<String> {
        // Hand-rolled split so we keep empty trailing cells (Kotlin's
        // String.split already does, but we want to avoid the regex overhead
        // for the hot path).
        val out = mutableListOf<String>()
        val cur = StringBuilder()
        for (c in line) {
            if (c == sep) {
                out += cur.toString()
                cur.setLength(0)
            } else {
                cur.append(c)
            }
        }
        out += cur.toString()
        return out
    }

    private fun normaliseRow(row: List<String>, width: Int): List<String> = when {
        row.size == width -> row
        row.size < width -> row + List(width - row.size) { "" }
        else -> row.take(width)
    }

    // -------------------------------------------------------------------------
    // Value parsing
    // -------------------------------------------------------------------------

    private val isoLocalDateTime: DateTimeFormatter = DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm:ss")
    private val isoLocalDate: DateTimeFormatter = DateTimeFormatter.ofPattern("yyyy-MM-dd")

    private fun resolveTimestamp(cells: List<String>, mode: TimestampMode): Long? = when (mode) {
        is TimestampMode.NowForAllRows -> System.currentTimeMillis()
        is TimestampMode.FromColumn -> {
            val raw = cells.getOrNull(mode.columnIndex).orEmpty().trim()
            parseTimestamp(raw)
        }
    }

    /**
     * Try four formats in order:
     *   1. `Instant.parse` (ISO 8601 with Z, e.g. "2026-05-12T14:00:00Z")
     *   2. `yyyy-MM-dd HH:mm:ss` (system zone)
     *   3. `yyyy-MM-dd` (midnight, system zone)
     *   4. plain Long (Unix milliseconds)
     */
    internal fun parseTimestamp(raw: String): Long? {
        if (raw.isEmpty()) return null
        runCatching { return Instant.parse(raw).toEpochMilli() }
        runCatching {
            val ldt = LocalDateTime.parse(raw, isoLocalDateTime)
            return ldt.atZone(ZoneId.systemDefault()).toInstant().toEpochMilli()
        }
        runCatching {
            val d = LocalDate.parse(raw, isoLocalDate)
            return d.atStartOfDay(ZoneId.systemDefault()).toInstant().toEpochMilli()
        }
        raw.toLongOrNull()?.let { return it }
        return null
    }

    private fun parseScalar(raw: String, type: ChannelType): OhdScalar? = when (type) {
        ChannelType.Real -> raw.replace(",", ".").toDoubleOrNull()?.let { OhdScalar.Real(it) }
        ChannelType.Int -> raw.toLongOrNull()?.let { OhdScalar.Int(it) }
        ChannelType.Bool -> parseBool(raw)?.let { OhdScalar.Bool(it) }
        ChannelType.Text -> OhdScalar.Text(raw)
    }

    private fun parseBool(raw: String): Boolean? = when (raw.lowercase()) {
        "true", "1", "yes", "y", "t" -> true
        "false", "0", "no", "n", "f" -> false
        else -> null
    }
}
