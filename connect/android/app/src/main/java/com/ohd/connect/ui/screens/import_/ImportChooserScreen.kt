package com.ohd.connect.ui.screens.import_

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Import chooser — entry point reached from Sources → "Import data".
 *
 * Presents one row per import format: a preset for Samsung Health Monitor
 * ECG CSV exports (fixed shape — no mapping needed), plus the generic CSV
 * and JSONL mappers that let the user pick column/path → event-channel
 * mappings interactively.
 */
@Composable
fun ImportChooserScreen(
    onBack: () -> Unit,
    onSamsungEcg: () -> Unit,
    onGenericCsv: () -> Unit,
    onGenericJsonl: () -> Unit,
    contentPadding: PaddingValues,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding)
            .verticalScroll(rememberScrollState()),
    ) {
        OhdTopBar(title = "Import data", onBack = onBack)

        Spacer(Modifier.height(8.dp))
        Text(
            text = "One-shot import from a file. The phone keeps the imported events locally; nothing leaves your device.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Muted,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 4.dp),
        )

        OhdSectionHeader("PRESETS")
        OhdListItem(
            primary = "Samsung Health Monitor — ECG",
            secondary = "Per-second waveform from Galaxy Watch ECG exports",
            meta = "›",
            onClick = onSamsungEcg,
        )

        OhdSectionHeader("GENERIC")
        OhdListItem(
            primary = "CSV with column mapping",
            secondary = "Map columns → event channels; pick a timestamp column",
            meta = "›",
            onClick = onGenericCsv,
        )
        OhdListItem(
            primary = "JSONL with path mapping",
            secondary = "Map JSON paths → event channels; one record per line",
            meta = "›",
            onClick = onGenericJsonl,
        )

        Spacer(Modifier.height(20.dp))
        Text(
            text = "Need another format? Open a GitHub issue and we'll add a preset.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            color = OhdColors.Muted,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
        )
        Spacer(Modifier.height(24.dp))
    }
}
