package com.ohd.connect.ui.screens.import_

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.ChannelType
import com.ohd.connect.data.ImportSummary
import com.ohd.connect.data.JsonlImporter
import com.ohd.connect.data.JsonlMapping
import com.ohd.connect.data.JsonlPreview
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdCard
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdMono
import kotlinx.coroutines.launch

/**
 * Generic JSONL importer. Three states mirror the CSV importer flow:
 *
 *  1. Picker — "Pick a JSONL file" button + brief description.
 *  2. Mapping — `event_type` field, timestamp source picker, per-path rows
 *     where the user chooses Skip / Channel / Timestamp. A 5-row preview
 *     table at the top shows what the parser saw.
 *  3. Done — emitted/error counts and an "Import another" button.
 *
 * No backward-compat wiring — the host wires this into `NavGraph.kt`.
 */
@Composable
fun ImportJsonlScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()

    var uri by remember { mutableStateOf<Uri?>(null) }
    var preview by remember { mutableStateOf<JsonlPreview?>(null) }
    var error by remember { mutableStateOf<String?>(null) }

    var eventType by remember { mutableStateOf("measurement.generic") }
    var timestampChoice by remember { mutableStateOf<String?>(null) } // null = NowForAllRecords
    var rowState by remember { mutableStateOf<List<JsonlRowState>>(emptyList()) }

    var working by remember { mutableStateOf(false) }
    var summary by remember { mutableStateOf<ImportSummary?>(null) }

    val picker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) { picked ->
        if (picked != null) {
            // Persist the URI so we can reopen the stream for the import pass
            // after the user finishes mapping.
            runCatching {
                ctx.contentResolver.takePersistableUriPermission(
                    picked,
                    android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION,
                )
            }
            uri = picked
            preview = null
            error = null
            summary = null
        }
    }

    // Run the preview pass once we have a URI.
    LaunchedEffect(uri) {
        val u = uri ?: return@LaunchedEffect
        working = true
        val res = runCatching {
            ctx.contentResolver.openInputStream(u)
                ?: throw IllegalStateException("Couldn't open file stream.")
        }.mapCatching { stream ->
            JsonlImporter.preview(stream).getOrThrow().also { stream.close() }
        }
        working = false
        res
            .onSuccess { p ->
                preview = p
                rowState = p.paths.map { rowStateFromMode(it, defaultModeFor(it, p)) }
                timestampChoice = p.paths.firstOrNull { looksLikeTimestamp(it) }
            }
            .onFailure { error = it.message ?: it::class.simpleName.orEmpty() }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Import JSONL", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            when {
                summary != null -> DoneSection(
                    summary = summary!!,
                    onReset = {
                        uri = null
                        preview = null
                        rowState = emptyList()
                        timestampChoice = null
                        summary = null
                        error = null
                    },
                )

                uri == null || preview == null -> PickerSection(
                    onPick = { picker.launch(arrayOf("application/*", "text/*")) },
                    working = working,
                    error = error,
                )

                else -> MappingSection(
                    preview = preview!!,
                    eventType = eventType,
                    onEventTypeChange = { eventType = it },
                    timestampChoice = timestampChoice,
                    onTimestampChoice = { timestampChoice = it },
                    rows = rowState,
                    onRowChange = { idx, row ->
                        rowState = rowState.toMutableList().also { it[idx] = row }
                    },
                    working = working,
                    error = error,
                    onImport = onImport@{
                        val u = uri ?: return@onImport
                        val mappings = rowState.map { it.toMapping(timestampChoice) }
                        working = true
                        scope.launch {
                            val res = runCatching {
                                ctx.contentResolver.openInputStream(u)
                                    ?: throw IllegalStateException("Couldn't open file stream.")
                            }.mapCatching { stream ->
                                val tsMode = timestampChoice
                                    ?.let { JsonlImporter.TimestampMode.FromPath(it) }
                                    ?: JsonlImporter.TimestampMode.NowForAllRecords
                                JsonlImporter.import(
                                    stream = stream,
                                    eventType = eventType.trim(),
                                    timestampMode = tsMode,
                                    mappings = mappings,
                                ).also { stream.close() }
                            }
                            working = false
                            res
                                .onSuccess { summary = it }
                                .onFailure { error = it.message ?: it::class.simpleName.orEmpty() }
                        }
                    },
                )
            }
        }
    }
}

// =============================================================================
// Sections
// =============================================================================

@Composable
private fun PickerSection(
    onPick: () -> Unit,
    working: Boolean,
    error: String?,
) {
    OhdCard(title = "Pick a JSONL file") {
        Text(
            text = "One JSON object per line. Nested fields flatten to dotted paths. " +
                "You'll see the first 5 records and map each path to a channel.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Muted,
            lineHeight = 18.sp,
        )
        Spacer(Modifier.height(8.dp))
        OhdButton(
            label = if (working) "Reading…" else "Pick a JSONL file",
            onClick = onPick,
            enabled = !working,
            modifier = Modifier.fillMaxWidth(),
        )
        if (error != null) {
            Spacer(Modifier.height(4.dp))
            Text(
                text = error,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.RedDark,
            )
        }
    }
}

@Composable
private fun MappingSection(
    preview: JsonlPreview,
    eventType: String,
    onEventTypeChange: (String) -> Unit,
    timestampChoice: String?,
    onTimestampChoice: (String?) -> Unit,
    rows: List<JsonlRowState>,
    onRowChange: (Int, JsonlRowState) -> Unit,
    working: Boolean,
    error: String?,
    onImport: () -> Unit,
) {
    OhdSectionHeader(text = "PREVIEW (${preview.firstRecords.size} of ${preview.totalRecordEstimate ?: "100+"} records)")
    PreviewTable(preview)

    OhdSectionHeader(text = "EVENT")
    OhdField(
        label = "event_type",
        value = eventType,
        onValueChange = onEventTypeChange,
        placeholder = "measurement.generic",
        helper = "Dotted path, e.g. measurement.glucose / food.eaten.",
    )

    OhdSectionHeader(text = "TIMESTAMP")
    TimestampPicker(
        paths = preview.paths,
        choice = timestampChoice,
        onChoice = onTimestampChoice,
    )

    OhdSectionHeader(text = "MAPPINGS")
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        rows.forEachIndexed { idx, row ->
            MappingRow(
                row = row,
                onChange = { onRowChange(idx, it) },
                isTimestampSource = (row.path == timestampChoice),
            )
        }
    }

    Spacer(Modifier.height(8.dp))
    val canImport = !working &&
        eventType.isNotBlank() &&
        rows.any { it.action == RowAction.Channel }
    OhdButton(
        label = if (working) "Importing…" else "Import",
        onClick = onImport,
        enabled = canImport,
        modifier = Modifier.fillMaxWidth(),
    )
    if (error != null) {
        Text(
            text = error,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            color = OhdColors.RedDark,
        )
    }
}

@Composable
private fun DoneSection(summary: ImportSummary, onReset: () -> Unit) {
    OhdCard(title = "Done") {
        Text(
            text = "Imported ${summary.emitted}. Errors: ${summary.errors}.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 14.sp,
            color = OhdColors.Ink,
        )
        if (summary.firstError != null) {
            Text(
                text = "First error: ${summary.firstError}",
                fontFamily = OhdMono,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
        }
        Spacer(Modifier.height(8.dp))
        OhdButton(
            label = "Import another",
            onClick = onReset,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

// =============================================================================
// Sub-components
// =============================================================================

@Composable
private fun PreviewTable(preview: JsonlPreview) {
    if (preview.firstRecords.isEmpty()) {
        Text(
            text = "No records parsed. The file may be empty or malformed.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Muted,
        )
        return
    }
    val cols = preview.paths
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .border(BorderStroke(1.dp, OhdColors.LineSoft), RoundedCornerShape(8.dp))
            .horizontalScroll(rememberScrollState()),
    ) {
        // Header row.
        Row(modifier = Modifier.background(OhdColors.BgElevated)) {
            cols.forEach { col ->
                TableCell(text = col, header = true)
            }
        }
        OhdDividerFlush()
        preview.firstRecords.forEachIndexed { idx, rec ->
            if (idx > 0) OhdDividerFlush()
            Row {
                cols.forEach { col ->
                    TableCell(text = rec[col]?.toString() ?: "—", header = false)
                }
            }
        }
    }
}

@Composable
private fun TableCell(text: String, header: Boolean) {
    Box(
        modifier = Modifier
            .width(140.dp)
            .padding(horizontal = 10.dp, vertical = 8.dp),
    ) {
        Text(
            text = text,
            fontFamily = if (header) OhdBody else OhdMono,
            fontWeight = if (header) FontWeight.W600 else FontWeight.W400,
            fontSize = if (header) 12.sp else 12.sp,
            color = if (header) OhdColors.Ink else OhdColors.InkSoft,
            maxLines = 2,
        )
    }
}

/** A 1 dp horizontal hairline that fills the parent table width (no insets). */
@Composable
private fun OhdDividerFlush() {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(1.dp)
            .background(OhdColors.LineSoft),
    )
}

@Composable
private fun TimestampPicker(
    paths: List<String>,
    choice: String?,
    onChoice: (String?) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Chip(
            label = "Now (for every row)",
            selected = choice == null,
            onClick = { onChoice(null) },
        )
        paths.forEach { p ->
            Chip(
                label = "From: $p",
                selected = choice == p,
                onClick = { onChoice(p) },
            )
        }
    }
}

@Composable
private fun Chip(label: String, selected: Boolean, onClick: () -> Unit) {
    val shape = RoundedCornerShape(8.dp)
    val mod = if (selected) {
        Modifier.background(OhdColors.Ink, shape)
    } else {
        Modifier
            .background(OhdColors.Bg, shape)
            .border(BorderStroke(1.dp, OhdColors.Line), shape)
    }
    Box(
        modifier = mod
            .fillMaxWidth()
            .clickable { onClick() }
            .padding(horizontal = 12.dp, vertical = 10.dp),
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = if (selected) FontWeight.W500 else FontWeight.W400,
            fontSize = 13.sp,
            color = if (selected) OhdColors.Bg else OhdColors.Ink,
        )
    }
}

@Composable
private fun MappingRow(
    row: JsonlRowState,
    onChange: (JsonlRowState) -> Unit,
    isTimestampSource: Boolean,
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .border(BorderStroke(1.dp, OhdColors.LineSoft), RoundedCornerShape(8.dp))
            .padding(12.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = row.path,
            fontFamily = OhdMono,
            fontWeight = FontWeight.W500,
            fontSize = 13.sp,
            color = OhdColors.Ink,
        )

        // Action row: Skip / Channel / Timestamp. If the timestamp source
        // picker already targets this path, force-disable Channel and show
        // the "Timestamp" pill in a selected state for clarity.
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            val action = if (isTimestampSource) RowAction.Timestamp else row.action
            SmallChip(
                label = "Skip",
                selected = action == RowAction.Skip,
                onClick = { onChange(row.copy(action = RowAction.Skip)) },
            )
            SmallChip(
                label = "Channel",
                selected = action == RowAction.Channel,
                onClick = { onChange(row.copy(action = RowAction.Channel)) },
            )
            SmallChip(
                label = "Timestamp",
                selected = action == RowAction.Timestamp,
                onClick = { onChange(row.copy(action = RowAction.Timestamp)) },
            )
        }

        if (row.action == RowAction.Channel && !isTimestampSource) {
            OhdField(
                label = "channel path",
                value = row.channelPath,
                onValueChange = { onChange(row.copy(channelPath = it)) },
                placeholder = row.path,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                ChannelType.entries.forEach { t ->
                    SmallChip(
                        label = t.name,
                        selected = row.type == t,
                        onClick = { onChange(row.copy(type = t)) },
                    )
                }
            }
            OhdField(
                label = "unit (optional)",
                value = row.unit,
                onValueChange = { onChange(row.copy(unit = it)) },
                placeholder = "mmol/L",
            )
        }
    }
}

@Composable
private fun SmallChip(label: String, selected: Boolean, onClick: () -> Unit) {
    val shape = RoundedCornerShape(8.dp)
    val mod = if (selected) {
        Modifier.background(OhdColors.Ink, shape)
    } else {
        Modifier
            .background(OhdColors.Bg, shape)
            .border(BorderStroke(1.dp, OhdColors.Line), shape)
    }
    Box(
        modifier = mod
            .height(32.dp)
            .clickable { onClick() }
            .padding(horizontal = 12.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = if (selected) FontWeight.W500 else FontWeight.W400,
            fontSize = 12.sp,
            color = if (selected) OhdColors.Bg else OhdColors.Ink,
        )
    }
}

// =============================================================================
// Row-state helpers
// =============================================================================

private enum class RowAction { Skip, Channel, Timestamp }

private data class JsonlRowState(
    val path: String,
    val action: RowAction = RowAction.Skip,
    val channelPath: String = path,
    val type: ChannelType = ChannelType.Text,
    val unit: String = "",
) {
    /**
     * Build the mapping the importer wants. The screen's timestamp picker
     * is authoritative — if [timestampChoice] points at this row's path we
     * always emit a [JsonlMapping.Mode.Timestamp], regardless of the
     * per-row toggle.
     */
    fun toMapping(timestampChoice: String?): JsonlMapping {
        if (timestampChoice != null && timestampChoice == path) {
            return JsonlMapping(path, JsonlMapping.Mode.Timestamp)
        }
        return when (action) {
            RowAction.Skip -> JsonlMapping(path, JsonlMapping.Mode.Skip)
            RowAction.Timestamp -> JsonlMapping(path, JsonlMapping.Mode.Timestamp)
            RowAction.Channel -> JsonlMapping(
                path,
                JsonlMapping.Mode.Channel(
                    channelPath = channelPath.ifBlank { path },
                    type = type,
                    unit = unit.ifBlank { null },
                ),
            )
        }
    }
}

/**
 * Pick a sensible default mapping based on the path name and the first
 * sample value. Numeric-looking values default to Real, booleans to Bool,
 * timestamp-looking paths get hidden behind the dedicated picker.
 */
private fun defaultModeFor(path: String, preview: JsonlPreview): JsonlMapping.Mode {
    if (looksLikeTimestamp(path)) return JsonlMapping.Mode.Skip
    val sample = preview.firstRecords.firstOrNull { it[path] != null }?.get(path) ?: return JsonlMapping.Mode.Skip
    val type = when (sample) {
        is Boolean -> ChannelType.Bool
        is Number -> if (sample.toDouble() % 1.0 == 0.0) ChannelType.Int else ChannelType.Real
        else -> ChannelType.Text
    }
    return JsonlMapping.Mode.Channel(channelPath = path, type = type, unit = null)
}

private fun looksLikeTimestamp(path: String): Boolean {
    val p = path.lowercase()
    return p == "ts" || p == "timestamp" || p == "time" || p == "date" ||
        p.endsWith(".ts") || p.endsWith(".timestamp") || p.endsWith(".time") || p.endsWith(".date")
}

/** Build the initial [JsonlRowState] for a discovered path given a default mode. */
private fun rowStateFromMode(path: String, mode: JsonlMapping.Mode): JsonlRowState = JsonlRowState(
    path = path,
    action = when (mode) {
        is JsonlMapping.Mode.Skip -> RowAction.Skip
        is JsonlMapping.Mode.Timestamp -> RowAction.Timestamp
        is JsonlMapping.Mode.Channel -> RowAction.Channel
    },
    channelPath = when (mode) {
        is JsonlMapping.Mode.Channel -> mode.channelPath
        else -> path
    },
    type = when (mode) {
        is JsonlMapping.Mode.Channel -> mode.type
        else -> ChannelType.Text
    },
    unit = when (mode) {
        is JsonlMapping.Mode.Channel -> mode.unit.orEmpty()
        else -> ""
    },
)
