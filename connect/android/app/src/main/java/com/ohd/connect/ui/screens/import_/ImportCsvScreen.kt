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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
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
import com.ohd.connect.data.CsvColumnMapping
import com.ohd.connect.data.CsvImporter
import com.ohd.connect.data.CsvPreview
import com.ohd.connect.data.ImportSummary
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdCard
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdMono
import kotlinx.coroutines.launch

/**
 * Generic CSV importer.
 *
 * Three-state single-screen flow:
 *   1. [Stage.Picker]  — explainer + "Pick a CSV" button.
 *   2. [Stage.Mapping] — event type, timestamp source, per-column mapping,
 *                       5-row monospace preview.
 *   3. [Stage.Done]    — green panel with import summary + "Import another".
 *
 * Wired into the navigation graph at the caller's site; this composable just
 * needs [contentPadding] and [onBack].
 */
@Composable
fun ImportCsvScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()

    var stage by remember { mutableStateOf<Stage>(Stage.Picker) }
    var status by remember { mutableStateOf<String?>(null) }
    var working by remember { mutableStateOf(false) }

    val picker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument(),
    ) launcher@{ uri: Uri? ->
        if (uri == null) return@launcher
        working = true
        status = null
        scope.launch {
            val name = queryDisplayName(ctx, uri) ?: "import.csv"
            val stream = runCatching { ctx.contentResolver.openInputStream(uri) }
                .getOrNull()
            if (stream == null) {
                working = false
                status = "Couldn't open file."
                return@launch
            }
            val result = CsvImporter.preview(stream)
            working = false
            result
                .onSuccess { preview ->
                    stage = Stage.Mapping(
                        uri = uri,
                        filename = name,
                        preview = preview,
                        eventType = filenameToEventType(name),
                        timestampMode = TimestampPick.Now,
                        mappings = preview.headers.mapIndexed { idx, h ->
                            EditableMapping(
                                columnIndex = idx,
                                header = h,
                                mode = EditableMode.Skip,
                                channelPath = slugify(h),
                                channelType = ChannelType.Real,
                                unit = "",
                            )
                        },
                    )
                }
                .onFailure { e ->
                    status = "Couldn't read CSV: ${e.message ?: "unknown error"}"
                }
        }
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Import CSV", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            when (val s = stage) {
                is Stage.Picker -> PickerStage(
                    working = working,
                    status = status,
                    onPick = { picker.launch(arrayOf("text/csv", "text/comma-separated-values", "*/*")) },
                )

                is Stage.Mapping -> MappingStage(
                    state = s,
                    working = working,
                    status = status,
                    onStateChange = { stage = it },
                    onCancel = {
                        stage = Stage.Picker
                        status = null
                    },
                    onImport = onImport@{
                        if (s.eventType.isBlank()) {
                            status = "Pick an event type."
                            return@onImport
                        }
                        working = true
                        status = null
                        scope.launch {
                            val stream = runCatching { ctx.contentResolver.openInputStream(s.uri) }
                                .getOrNull()
                            if (stream == null) {
                                working = false
                                status = "Couldn't re-open file for import."
                                return@launch
                            }
                            val tsMode = when (val tm = s.timestampMode) {
                                TimestampPick.Now -> CsvImporter.TimestampMode.NowForAllRows
                                is TimestampPick.FromColumn -> CsvImporter.TimestampMode.FromColumn(tm.columnIndex)
                            }
                            val coreMappings = s.mappings.mapNotNull { it.toCore() }
                            val summary = CsvImporter.import(
                                stream = stream,
                                eventType = s.eventType.trim(),
                                timestampMode = tsMode,
                                mappings = coreMappings,
                                source = "import:csv",
                            )
                            working = false
                            stage = Stage.Done(summary = summary)
                        }
                    },
                )

                is Stage.Done -> DoneStage(
                    summary = s.summary,
                    onAgain = {
                        stage = Stage.Picker
                        status = null
                    },
                )
            }
        }
    }
}

// =============================================================================
// Stages
// =============================================================================

private sealed interface Stage {
    data object Picker : Stage
    data class Mapping(
        val uri: Uri,
        val filename: String,
        val preview: CsvPreview,
        val eventType: String,
        val timestampMode: TimestampPick,
        val mappings: List<EditableMapping>,
    ) : Stage

    data class Done(val summary: ImportSummary) : Stage
}

private sealed interface TimestampPick {
    data object Now : TimestampPick
    data class FromColumn(val columnIndex: Int) : TimestampPick
}

private enum class EditableMode { Skip, Channel, Timestamp }

private data class EditableMapping(
    val columnIndex: Int,
    val header: String,
    val mode: EditableMode,
    val channelPath: String,
    val channelType: ChannelType,
    val unit: String,
) {
    fun toCore(): CsvColumnMapping? = when (mode) {
        EditableMode.Skip -> null
        EditableMode.Timestamp -> CsvColumnMapping(
            columnIndex = columnIndex,
            mode = CsvColumnMapping.Mode.Timestamp,
        )
        EditableMode.Channel -> CsvColumnMapping(
            columnIndex = columnIndex,
            mode = CsvColumnMapping.Mode.Channel(
                path = channelPath.ifBlank { slugify(header) },
                type = channelType,
                unit = unit.takeIf { it.isNotBlank() },
            ),
        )
    }
}

// =============================================================================
// Picker stage — file picker entry point
// =============================================================================

@Composable
private fun PickerStage(
    working: Boolean,
    status: String?,
    onPick: () -> Unit,
) {
    OhdCard(title = "Import a CSV") {
        Text(
            text = "Pick a .csv from your device. You'll see the first five rows and pick which column maps to the timestamp and which become event channels.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Muted,
        )
        Spacer(Modifier.height(4.dp))
        if (working) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(18.dp),
                    color = OhdColors.Ink,
                    strokeWidth = 2.dp,
                )
                Text(
                    text = "Reading preview…",
                    fontFamily = OhdBody,
                    fontSize = 13.sp,
                    color = OhdColors.Muted,
                )
            }
        } else {
            OhdButton(label = "Pick a CSV", onClick = onPick)
        }
        if (status != null) {
            Text(
                text = status,
                fontFamily = OhdBody,
                fontSize = 12.sp,
                color = OhdColors.Red,
            )
        }
    }
}

// =============================================================================
// Mapping stage — event type, timestamp source, per-column mapping, preview
// =============================================================================

@Composable
private fun MappingStage(
    state: Stage.Mapping,
    working: Boolean,
    status: String?,
    onStateChange: (Stage.Mapping) -> Unit,
    onCancel: () -> Unit,
    onImport: () -> Unit,
) {
    // Auto-clear the warning when the user has at least one channel mapping
    // and a valid timestamp source — purely cosmetic, the import call also
    // re-validates.
    val hasChannel = state.mappings.any { it.mode == EditableMode.Channel }
    val hasTsColumn = state.mappings.any { it.mode == EditableMode.Timestamp }
    val tsModeValid = when (state.timestampMode) {
        TimestampPick.Now -> true
        is TimestampPick.FromColumn -> hasTsColumn
    }

    OhdCard(title = state.filename) {
        Text(
            text = "${state.preview.headers.size} columns · ${state.preview.firstRows.size} preview rows",
            fontFamily = OhdBody,
            fontSize = 12.sp,
            color = OhdColors.Muted,
        )
    }

    // 1. Event type --------------------------------------------------------
    OhdSectionHeader(text = "EVENT TYPE")
    OhdCard {
        OhdField(
            label = "event_type",
            value = state.eventType,
            onValueChange = { onStateChange(state.copy(eventType = it)) },
            placeholder = "measurement.glucose",
            helper = "Free-form. The storage core validates the name.",
        )
    }

    // 2. Timestamp source --------------------------------------------------
    OhdSectionHeader(text = "TIMESTAMP")
    OhdCard {
        TimestampPicker(
            state = state,
            onPick = { tm ->
                // When switching to FromColumn, sync the matching column's
                // mode to Timestamp so the table reflects the choice.
                val newMappings = when (tm) {
                    TimestampPick.Now -> state.mappings.map {
                        if (it.mode == EditableMode.Timestamp) it.copy(mode = EditableMode.Skip) else it
                    }
                    is TimestampPick.FromColumn -> state.mappings.map {
                        when {
                            it.columnIndex == tm.columnIndex -> it.copy(mode = EditableMode.Timestamp)
                            it.mode == EditableMode.Timestamp -> it.copy(mode = EditableMode.Skip)
                            else -> it
                        }
                    }
                }
                onStateChange(state.copy(timestampMode = tm, mappings = newMappings))
            },
        )
    }

    // 3. Per-column mapping ------------------------------------------------
    OhdSectionHeader(text = "COLUMNS")
    OhdCard {
        state.mappings.forEachIndexed { idx, m ->
            ColumnMappingRow(
                mapping = m,
                onChange = { updated ->
                    val newMappings = state.mappings.toMutableList().also { it[idx] = updated }
                    // If user flipped this row to Timestamp, also lift the
                    // top-level timestamp mode + clear any other timestamp.
                    val newTsMode: TimestampPick = if (updated.mode == EditableMode.Timestamp) {
                        TimestampPick.FromColumn(updated.columnIndex)
                    } else if (state.timestampMode is TimestampPick.FromColumn &&
                        state.timestampMode.columnIndex == updated.columnIndex
                    ) {
                        TimestampPick.Now
                    } else {
                        state.timestampMode
                    }
                    val deduped = if (updated.mode == EditableMode.Timestamp) {
                        newMappings.mapIndexed { i, x ->
                            if (i != idx && x.mode == EditableMode.Timestamp) {
                                x.copy(mode = EditableMode.Skip)
                            } else {
                                x
                            }
                        }
                    } else {
                        newMappings
                    }
                    onStateChange(state.copy(mappings = deduped, timestampMode = newTsMode))
                },
            )
            if (idx < state.mappings.lastIndex) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(1.dp)
                        .background(OhdColors.LineSoft),
                )
            }
        }
    }

    // 4. Preview rows ------------------------------------------------------
    OhdSectionHeader(text = "PREVIEW")
    OhdCard {
        PreviewTable(state.preview)
    }

    // 5. CTAs --------------------------------------------------------------
    if (status != null) {
        Text(
            text = status,
            fontFamily = OhdBody,
            fontSize = 12.sp,
            color = OhdColors.Red,
        )
    }
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        OhdButton(
            label = "Cancel",
            onClick = onCancel,
            variant = OhdButtonVariant.Secondary,
            enabled = !working,
            modifier = Modifier.weight(1f),
        )
        if (working) {
            Row(
                modifier = Modifier.weight(1f).height(40.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
            ) {
                CircularProgressIndicator(
                    modifier = Modifier.size(18.dp),
                    color = OhdColors.Red,
                    strokeWidth = 2.dp,
                )
                Text(
                    text = "Importing…",
                    fontFamily = OhdBody,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.W500,
                    color = OhdColors.Muted,
                )
            }
        } else {
            OhdButton(
                label = "Import",
                onClick = onImport,
                enabled = hasChannel && tsModeValid && state.eventType.isNotBlank(),
                modifier = Modifier.weight(1f),
            )
        }
    }
}

@Composable
private fun TimestampPicker(state: Stage.Mapping, onPick: (TimestampPick) -> Unit) {
    val current = state.timestampMode
    val options = buildList {
        add(TimestampPick.Now to "Now (for each row)")
        state.preview.headers.forEachIndexed { idx, h ->
            add(TimestampPick.FromColumn(idx) to "From column: $h")
        }
    }
    val currentLabel = when (current) {
        TimestampPick.Now -> "Now (for each row)"
        is TimestampPick.FromColumn -> "From column: ${state.preview.headers.getOrNull(current.columnIndex) ?: "?"}"
    }
    DropdownField(
        label = "Source",
        currentLabel = currentLabel,
        options = options.map { it.second },
        onSelect = { idx -> onPick(options[idx].first) },
    )
}

@Composable
private fun ColumnMappingRow(
    mapping: EditableMapping,
    onChange: (EditableMapping) -> Unit,
) {
    Column(
        modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        // Column header line — index + name.
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "#${mapping.columnIndex + 1}",
                fontFamily = OhdMono,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
            Text(
                text = mapping.header.ifBlank { "(empty header)" },
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 14.sp,
                color = OhdColors.Ink,
                modifier = Modifier.weight(1f),
            )
        }

        // Mode picker — Skip / Channel / Timestamp.
        val modeLabels = listOf("Skip", "Channel", "Timestamp")
        val modeValues = listOf(EditableMode.Skip, EditableMode.Channel, EditableMode.Timestamp)
        val currentModeIdx = modeValues.indexOf(mapping.mode)
        DropdownField(
            label = "Map to",
            currentLabel = modeLabels[currentModeIdx],
            options = modeLabels,
            onSelect = { i -> onChange(mapping.copy(mode = modeValues[i])) },
        )

        if (mapping.mode == EditableMode.Channel) {
            // Type picker.
            val typeLabels = listOf("Real", "Int", "Bool", "Text")
            val typeValues = listOf(ChannelType.Real, ChannelType.Int, ChannelType.Bool, ChannelType.Text)
            val currentTypeIdx = typeValues.indexOf(mapping.channelType)
            DropdownField(
                label = "Type",
                currentLabel = typeLabels[currentTypeIdx],
                options = typeLabels,
                onSelect = { i -> onChange(mapping.copy(channelType = typeValues[i])) },
            )
            OhdField(
                label = "Channel path",
                value = mapping.channelPath,
                onValueChange = { onChange(mapping.copy(channelPath = it)) },
                placeholder = slugify(mapping.header),
            )
            OhdField(
                label = "Unit (optional)",
                value = mapping.unit,
                onValueChange = { onChange(mapping.copy(unit = it)) },
                placeholder = "e.g. mmol/L",
            )
        }
    }
}

@Composable
private fun DropdownField(
    label: String,
    currentLabel: String,
    options: List<String>,
    onSelect: (Int) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 13.sp,
            color = OhdColors.Ink,
        )
        Box {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(44.dp)
                    .background(OhdColors.Bg, RoundedCornerShape(8.dp))
                    .border(BorderStroke(1.5.dp, OhdColors.Line), RoundedCornerShape(8.dp))
                    .clickable { expanded = true }
                    .padding(horizontal = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    text = currentLabel,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 14.sp,
                    color = OhdColors.Ink,
                    modifier = Modifier.weight(1f),
                )
                Icon(
                    imageVector = OhdIcons.ChevronDown,
                    contentDescription = null,
                    tint = OhdColors.Muted,
                    modifier = Modifier.size(18.dp),
                )
            }
            DropdownMenu(
                expanded = expanded,
                onDismissRequest = { expanded = false },
            ) {
                options.forEachIndexed { i, opt ->
                    DropdownMenuItem(
                        text = {
                            Text(
                                text = opt,
                                fontFamily = OhdBody,
                                fontSize = 14.sp,
                                color = OhdColors.Ink,
                            )
                        },
                        onClick = {
                            expanded = false
                            onSelect(i)
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun PreviewTable(preview: CsvPreview) {
    val scroll = rememberScrollState()
    val colWidth = 120.dp
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .horizontalScroll(scroll),
    ) {
        // Header row.
        Row(modifier = Modifier.padding(vertical = 4.dp)) {
            preview.headers.forEach { h ->
                Text(
                    text = h,
                    fontFamily = OhdMono,
                    fontWeight = FontWeight.W500,
                    fontSize = 11.sp,
                    color = OhdColors.Ink,
                    modifier = Modifier
                        .width(colWidth)
                        .padding(end = 8.dp),
                    maxLines = 1,
                )
            }
        }
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(OhdColors.LineSoft),
        )
        // Data rows.
        preview.firstRows.forEachIndexed { rowIdx, row ->
            Row(modifier = Modifier.padding(vertical = 4.dp)) {
                row.forEach { cell ->
                    Text(
                        text = cell,
                        fontFamily = OhdMono,
                        fontWeight = FontWeight.W400,
                        fontSize = 11.sp,
                        color = OhdColors.Muted,
                        modifier = Modifier
                            .width(colWidth)
                            .padding(end = 8.dp),
                        maxLines = 1,
                    )
                }
            }
            if (rowIdx < preview.firstRows.lastIndex) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(1.dp)
                        .background(OhdColors.LineSoft),
                )
            }
        }
        if (preview.firstRows.isEmpty()) {
            Text(
                text = "(no data rows)",
                fontFamily = OhdBody,
                fontSize = 12.sp,
                color = OhdColors.Muted,
                modifier = Modifier.padding(vertical = 8.dp),
            )
        }
    }
}

// =============================================================================
// Done stage — success panel
// =============================================================================

@Composable
private fun DoneStage(summary: ImportSummary, onAgain: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(OhdColors.Success.copy(alpha = 0.10f), RoundedCornerShape(12.dp))
            .border(BorderStroke(1.dp, OhdColors.Success.copy(alpha = 0.40f)), RoundedCornerShape(12.dp))
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = "Imported ${summary.emitted} event${if (summary.emitted == 1) "" else "s"}. ${summary.errors} row${if (summary.errors == 1) "" else "s"} skipped.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W600,
            fontSize = 15.sp,
            color = OhdColors.Success,
        )
        if (summary.firstError != null) {
            Text(
                text = "First skipped: ${summary.firstError}",
                fontFamily = OhdBody,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
        }
    }
    OhdButton(label = "Import another", onClick = onAgain)
}

// =============================================================================
// Helpers
// =============================================================================

/** Strip extension, lowercase, replace non-alnum with `_`, collapse repeats. */
private fun filenameToEventType(name: String): String {
    val base = name.substringBeforeLast('.', name)
    return slugify(base).ifBlank { "import.csv" }
}

private fun slugify(s: String): String {
    val cleaned = s.trim().lowercase().map { if (it.isLetterOrDigit()) it else '_' }.joinToString("")
    return cleaned.trim('_').replace(Regex("_+"), "_")
}

/** Best-effort display name from a content-Uri (SAF). Falls back to URI tail. */
private fun queryDisplayName(ctx: android.content.Context, uri: Uri): String? {
    return runCatching {
        ctx.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
            val idx = cursor.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
            if (idx >= 0 && cursor.moveToFirst()) cursor.getString(idx) else null
        }
    }.getOrNull() ?: uri.lastPathSegment
}
