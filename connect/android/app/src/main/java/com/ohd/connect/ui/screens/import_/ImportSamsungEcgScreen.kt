package com.ohd.connect.ui.screens.import_

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import android.widget.Toast
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.documentfile.provider.DocumentFile
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.toMutableStateList
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.SamsungEcgImporter
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdCard
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdMono
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.runInterruptible
import kotlinx.coroutines.withContext

/**
 * Per-row state for a picked Samsung ECG CSV. Tracks the lifecycle from
 * picked → parsing → parsed/duplicate/failed → importing → imported/error.
 */
private sealed interface EcgRowState {
    data object Picked : EcgRowState
    data object Parsing : EcgRowState
    data class Parsed(
        val durationSec: Double,
        val avgHr: Double?,
        val classification: String?,
        val meta: SamsungEcgImporter.StripMetadata,
        val samples: FloatArray,
    ) : EcgRowState
    data class Imported(val secondsEmitted: Int) : EcgRowState
    data object Duplicate : EcgRowState
    data class Failed(val message: String) : EcgRowState
    data object Importing : EcgRowState
}

private data class EcgRow(
    val uri: Uri,
    val filename: String,
    val state: EcgRowState,
)

/**
 * Import Samsung Health Monitor ECG CSVs from the user's Downloads folder.
 *
 * Uses [ActivityResultContracts.OpenMultipleDocuments] to pick one or more
 * files, parses each via [SamsungEcgImporter.parse], then on the "Import
 * all" CTA emits one `measurement.ecg_second` event per second of waveform
 * for each strip. Re-imports of an already-stored strip are detected by
 * `correlation_id` lookup and surfaced as "Already imported".
 */
@Composable
fun ImportSamsungEcgScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val rows = remember { mutableListOf<EcgRow>().toMutableStateList() }
    var importing by remember { mutableStateOf(false) }

    // Folder picker — Samsung's Download_ECG_<timestamp>/ folders contain
    // one CSV per strip, so the user picks the directory and we enumerate.
    val launcher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.OpenDocumentTree(),
    ) { treeUri: Uri? ->
        if (treeUri == null) return@rememberLauncherForActivityResult
        val newRows = enumerateCsvs(ctx, treeUri)
        if (newRows.isEmpty()) {
            Toast.makeText(ctx, "No CSV files found in that folder", Toast.LENGTH_SHORT).show()
            return@rememberLauncherForActivityResult
        }
        rows.addAll(newRows)
    }

    // Kick off parsing for any newly-picked rows. The LaunchedEffect re-runs
    // when rows.size grows so multiple file batches each get parsed.
    LaunchedEffect(rows.size) {
        rows.forEachIndexed { idx, row ->
            if (row.state !is EcgRowState.Picked) return@forEachIndexed
            rows[idx] = row.copy(state = EcgRowState.Parsing)
            scope.launch {
                val parsed = runInterruptible(Dispatchers.IO) {
                    ctx.contentResolver.openInputStream(row.uri)?.use {
                        SamsungEcgImporter.parse(it)
                    } ?: Result.failure(IllegalStateException("cannot open ${row.uri}"))
                }
                val newState: EcgRowState = parsed.fold(
                    onSuccess = { (meta, samples) ->
                        EcgRowState.Parsed(
                            durationSec = meta.durationSec,
                            avgHr = meta.avgHeartRate,
                            classification = meta.classification,
                            meta = meta,
                            samples = samples,
                        )
                    },
                    onFailure = { e -> EcgRowState.Failed(e.message ?: "parse failed") },
                )
                val i = rows.indexOfFirst { it.uri == row.uri }
                if (i >= 0) rows[i] = rows[i].copy(state = newState)
            }
        }
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Import Samsung ECG", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            OhdCard(title = "How to export") {
                Text(
                    text = "On your phone, open the Samsung Health Monitor app → Profile → ECG → ⋮ → Export. The CSV files land in Downloads. Pick them below.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    color = OhdColors.Muted,
                    lineHeight = 19.sp,
                )
            }

            OhdButton(
                label = if (rows.isEmpty()) "Pick files" else "Pick more files",
                onClick = { launcher.launch(null) },
                modifier = Modifier.fillMaxWidth(),
                enabled = !importing,
            )

            if (rows.isNotEmpty()) {
                OhdSectionHeader(text = "FILES")
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    rows.forEach { row ->
                        EcgRowItem(row)
                    }
                }
            }

            val readyCount = rows.count { it.state is EcgRowState.Parsed }
            val anyParsing = rows.any { it.state is EcgRowState.Parsing }
            Spacer(modifier = Modifier.height(8.dp))
            OhdButton(
                label = if (importing) "Importing…" else "Import all ($readyCount)",
                onClick = onClick@{
                    if (importing || readyCount == 0) return@onClick
                    importing = true
                    scope.launch {
                        var imported = 0
                        var totalSeconds = 0
                        rows.indices.forEach { i ->
                            val r = rows[i]
                            val parsed = r.state as? EcgRowState.Parsed ?: return@forEach
                            rows[i] = r.copy(state = EcgRowState.Importing)
                            val result = withContext(Dispatchers.IO) {
                                SamsungEcgImporter.importStrip(parsed.meta, parsed.samples)
                            }
                            val next: EcgRowState = when {
                                result.error != null -> EcgRowState.Failed(result.error)
                                result.skippedDuplicate -> EcgRowState.Duplicate
                                else -> {
                                    imported++
                                    totalSeconds += result.secondsEmitted
                                    EcgRowState.Imported(result.secondsEmitted)
                                }
                            }
                            val j = rows.indexOfFirst { it.uri == r.uri }
                            if (j >= 0) rows[j] = rows[j].copy(state = next)
                        }
                        importing = false
                        Toast.makeText(
                            ctx,
                            "Imported $imported strips ($totalSeconds seconds)",
                            Toast.LENGTH_LONG,
                        ).show()
                    }
                },
                modifier = Modifier.fillMaxWidth(),
                enabled = !importing && readyCount > 0 && !anyParsing,
            )
        }
    }
}

@Composable
private fun EcgRowItem(row: EcgRow) {
    val summary = when (val s = row.state) {
        EcgRowState.Picked, EcgRowState.Parsing -> "Parsing…"
        EcgRowState.Importing -> "Importing…"
        is EcgRowState.Parsed -> buildSummary(s)
        is EcgRowState.Imported -> "Imported (${s.secondsEmitted} s)"
        EcgRowState.Duplicate -> "Already imported"
        is EcgRowState.Failed -> "Failed: ${s.message}"
    }
    val summaryColor = when (row.state) {
        is EcgRowState.Failed -> OhdColors.Red
        is EcgRowState.Imported, EcgRowState.Duplicate -> OhdColors.Muted
        else -> OhdColors.Ink
    }
    OhdCard {
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(
                text = row.filename,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 13.sp,
                color = OhdColors.Ink,
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = summary,
                    fontFamily = OhdMono,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = summaryColor,
                )
            }
        }
    }
}

private fun buildSummary(s: EcgRowState.Parsed): String {
    val parts = mutableListOf<String>()
    parts += "${s.durationSec.toInt()} s"
    s.avgHr?.let { parts += "${it.toInt()} bpm" }
    s.classification?.let { parts += it }
    return parts.joinToString(" · ")
}

private fun queryDisplayName(ctx: Context, uri: Uri): String? {
    return ctx.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
        ?.use { cursor ->
            if (cursor.moveToFirst()) cursor.getString(0) else null
        }
}

/**
 * Walk a user-picked tree URI and collect every `*.csv` underneath it
 * (Samsung's `Download_ECG_<ts>/` folders have one CSV per strip but
 * users sometimes pick the parent — recurse to be safe).
 */
private fun enumerateCsvs(ctx: Context, treeUri: Uri): List<EcgRow> {
    val root = DocumentFile.fromTreeUri(ctx, treeUri) ?: return emptyList()
    val out = mutableListOf<EcgRow>()
    walk(root, out, depth = 0)
    return out
}

private fun walk(dir: DocumentFile, out: MutableList<EcgRow>, depth: Int) {
    if (depth > 4) return
    dir.listFiles().forEach { f ->
        when {
            f.isDirectory -> walk(f, out, depth + 1)
            f.isFile && (f.name?.endsWith(".csv", ignoreCase = true) == true) -> {
                out += EcgRow(
                    uri = f.uri,
                    filename = f.name ?: f.uri.lastPathSegment.orEmpty().ifBlank { "(unnamed)" },
                    state = EcgRowState.Picked,
                )
            }
        }
    }
}
