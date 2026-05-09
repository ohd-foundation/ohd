package com.ohd.connect.ui.screens

import android.graphics.Paint
import android.graphics.pdf.PdfDocument
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import com.ohd.connect.data.ExportRecord
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.theme.MonoStyle
import com.ohd.connect.ui.theme.OhdConnectTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.io.FileOutputStream

/**
 * Export / portability screen — surfaces the three export paths from
 * `connect/SPEC.md` "Export / portability":
 *
 *  1. Full lossless `.ohd` export — calls `StorageRepository.exportAll()`,
 *     which today writes a stub file but is shaped to swap one-for-one
 *     with `Export.Export` (server-streaming) once that uniffi binding
 *     ships. The file lives in `<filesDir>/exports/`; recent files appear
 *     in the history list at the bottom of the screen.
 *  2. Doctor PDF — generated client-side via Android's `PdfDocument` API
 *     for v0 (a one-page placeholder summary). When storage's
 *     `Export.GenerateDoctorPdf` ships, the repository swaps the
 *     rendering path; this screen needs no changes.
 *  3. Migration assistant — placeholder; documents the
 *     `Export.MigrateInit` / `Export.MigrateFinalize` flow. Disabled until
 *     storage exposes the RPCs.
 *
 * Repository TODOs flagged in `StorageRepository.kt`. UI is real.
 */
@Composable
fun ExportScreen(contentPadding: PaddingValues) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()

    var recents by remember { mutableStateOf<List<ExportRecord>>(emptyList()) }
    var lastResult by remember { mutableStateOf<String?>(null) }
    var working by remember { mutableStateOf(false) }
    var refreshTick by remember { mutableStateOf(0) }

    LaunchedEffect(refreshTick) {
        StorageRepository.listExports().onSuccess { recents = it }
    }

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(contentPadding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "Export / portability",
                style = MaterialTheme.typography.headlineSmall,
            )
            Text(
                text = "Take your data with you. Files land in this app's private storage; share via the system share sheet.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(4.dp))

            // --- Full lossless export -------------------------------------
            Card(
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surfaceVariant,
                ),
            ) {
                Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                    Text(text = "Full lossless export", style = MaterialTheme.typography.titleMedium)
                    Text(
                        text = "Stream every event, every channel value, every grant rule, every audit-log row, every attachment to a single signed `.ohd` file. Restorable to any OHD Storage instance via Import.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(8.dp))
                    Button(
                        onClick = {
                            working = true
                            scope.launch {
                                val res = withContext(Dispatchers.IO) {
                                    StorageRepository.exportAll()
                                }
                                working = false
                                res.onSuccess {
                                    lastResult = "Wrote $it"
                                    refreshTick++
                                }.onFailure {
                                    lastResult = "Export failed: ${it.message}"
                                }
                            }
                        },
                        enabled = !working,
                    ) { Text(if (working) "Working…" else "Export everything") }
                }
            }

            // --- Doctor PDF ----------------------------------------------
            Card(
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surfaceVariant,
                ),
            ) {
                Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                    Text(text = "Doctor PDF", style = MaterialTheme.typography.titleMedium)
                    Text(
                        text = "A curated PDF for in-person sharing — recent vitals, active medications, allergies. v0 renders client-side via Android's PdfDocument API; storage's Export.GenerateDoctorPdf takes over in v0.x for byte-identical layout across instances.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(8.dp))
                    Button(
                        onClick = {
                            working = true
                            scope.launch {
                                val res = withContext(Dispatchers.IO) {
                                    runCatching { writeDoctorPdfPlaceholder(ctx.filesDir) }
                                }
                                working = false
                                res.onSuccess {
                                    lastResult = "Wrote $it"
                                    refreshTick++
                                }.onFailure {
                                    lastResult = "PDF failed: ${it.message}"
                                }
                            }
                        },
                        enabled = !working,
                    ) { Text(if (working) "Working…" else "Generate doctor PDF") }
                }
            }

            // --- Migration assistant -------------------------------------
            Card(
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surfaceVariant,
                ),
            ) {
                Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                    Text(text = "Migrate to a different deployment mode", style = MaterialTheme.typography.titleMedium)
                    Text(
                        text = "Moving between deployment modes (on-device → cloud, cloud → self-hosted) is the two-step Export.MigrateInit → Export.MigrateFinalize flow. The new instance verifies the source-instance signature on the export, accepts the stream, and the source instance flips read-only for a finite cutover window. v0.x (storage RPC pending).",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(8.dp))
                    OutlinedButton(
                        onClick = { lastResult = "Migration assistant: TODO (storage RPC pending)" },
                        enabled = false,
                    ) { Text("Migrate (TBD)") }
                }
            }

            lastResult?.let { msg ->
                Text(
                    text = msg,
                    style = MonoStyle,
                    color = MaterialTheme.colorScheme.tertiary,
                )
            }

            Spacer(Modifier.height(8.dp))
            HorizontalDivider()

            // --- Recent exports ------------------------------------------
            Text(
                text = "Recent exports",
                style = MaterialTheme.typography.titleMedium,
            )
            if (recents.isEmpty()) {
                Text(
                    text = "No exports yet.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    recents.take(20).forEach { rec ->
                        Card(
                            colors = CardDefaults.cardColors(
                                containerColor = MaterialTheme.colorScheme.surfaceVariant,
                            ),
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Column(modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) {
                                Text(
                                    text = File(rec.absolutePath).name,
                                    style = MonoStyle,
                                )
                                Row(
                                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                                ) {
                                    Text(
                                        text = fmtDate(rec.createdAtMs),
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                    Text(
                                        text = humanSize(rec.sizeBytes),
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                            }
                        }
                    }
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}

/**
 * v0 doctor-PDF stub: writes a one-page A4 PDF with title + a "TODO real
 * PDF rendering" placeholder line. Swaps to the real layout in v0.x once
 * `StorageRepository.generateDoctorPdf()` returns a server-rendered file.
 */
private fun writeDoctorPdfPlaceholder(filesDir: File): String {
    val outDir = File(filesDir, "exports").apply { mkdirs() }
    val target = File(outDir, "ohd-doctor-${System.currentTimeMillis()}.pdf")
    val doc = PdfDocument()
    val pageInfo = PdfDocument.PageInfo.Builder(595, 842, 1).create()
    val page = doc.startPage(pageInfo)
    val canvas = page.canvas
    val titlePaint = Paint().apply {
        color = android.graphics.Color.BLACK
        textSize = 22f
        isAntiAlias = true
    }
    val bodyPaint = Paint().apply {
        color = android.graphics.Color.DKGRAY
        textSize = 12f
        isAntiAlias = true
    }
    val mono = Paint().apply {
        color = android.graphics.Color.DKGRAY
        textSize = 10f
        isAntiAlias = true
        typeface = android.graphics.Typeface.MONOSPACE
    }
    canvas.drawText("OHD Doctor Summary", 48f, 80f, titlePaint)
    canvas.drawText("Placeholder one-page export (v0 client-side render).", 48f, 110f, bodyPaint)
    canvas.drawText("Generated: ${java.util.Date()}", 48f, 130f, bodyPaint)
    canvas.drawText(
        "TODO: real PDF rendering once storage Export.GenerateDoctorPdf ships.",
        48f,
        160f,
        bodyPaint,
    )
    canvas.drawText("Source: connect/android ExportScreen.kt", 48f, 190f, mono)
    doc.finishPage(page)
    FileOutputStream(target).use { doc.writeTo(it) }
    doc.close()
    return target.absolutePath
}

private fun humanSize(bytes: Long): String {
    if (bytes < 1024) return "$bytes B"
    val k = bytes / 1024.0
    if (k < 1024) return String.format("%.1f KB", k)
    val m = k / 1024.0
    if (m < 1024) return String.format("%.1f MB", m)
    return String.format("%.1f GB", m / 1024.0)
}

@Preview(showBackground = true, heightDp = 720)
@Composable
private fun ExportScreenPreview() {
    OhdConnectTheme {
        Surface { ExportScreen(contentPadding = PaddingValues(0.dp)) }
    }
}
