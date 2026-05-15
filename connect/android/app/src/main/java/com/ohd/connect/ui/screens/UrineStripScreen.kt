package com.ohd.connect.ui.screens

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.EventChannelInput
import com.ohd.connect.data.EventInput
import com.ohd.connect.data.OhdScalar
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TopBarAction
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdMono

/**
 * The four built-in analytes captured by the urine strip flow.
 *
 * Per spec §4.9 the live form has 8 fields; v1 ships these four (Glucose,
 * pH, Protein, Leukocytes) which match the Pencil reference `N00Rs.png`.
 */
enum class UrineAnalyte { Glucose, PH, Protein, Leukocytes }

private data class AnalyteSpec(
    val analyte: UrineAnalyte,
    val name: String,
    val swatches: List<Color>,
    val captionLow: String? = null,
    val captionHigh: String? = null,
    val defaultIndex: Int?,
    val valueLabels: List<String>,
    val healthyValueIndex: Int? = null,
)

private val Analytes: List<AnalyteSpec> = listOf(
    AnalyteSpec(
        analyte = UrineAnalyte.Glucose,
        name = "Glucose",
        swatches = listOf(
            Color(0xFFF2F0DC),
            Color(0xFFD4E8A0),
            Color(0xFF8FC86E),
            Color(0xFF4A9E3C),
            Color(0xFF1E6B28),
        ),
        captionLow = "Neg",
        captionHigh = ">55",
        defaultIndex = 0,
        valueLabels = listOf("Negative", "Trace", "Mild", "Moderate", ">55"),
    ),
    AnalyteSpec(
        analyte = UrineAnalyte.PH,
        name = "pH",
        swatches = listOf(
            Color(0xFFFF7733),
            Color(0xFFFFAA44),
            Color(0xFFEEEE55),
            Color(0xFFAADDAA),
            Color(0xFF66BBAA),
            Color(0xFF4499CC),
        ),
        captionLow = "5",
        captionHigh = "9",
        defaultIndex = 3,
        valueLabels = listOf("5.0", "6.0", "6.5", "7.0", "8.0", "9.0"),
        healthyValueIndex = 3,
    ),
    AnalyteSpec(
        analyte = UrineAnalyte.Protein,
        name = "Protein",
        swatches = listOf(
            Color(0xFFF5F0DC),
            Color(0xFFE8D88A),
            Color(0xFFC8AA44),
            Color(0xFF886622),
        ),
        defaultIndex = 0,
        valueLabels = listOf("Negative", "Trace", "Mild", "High"),
    ),
    AnalyteSpec(
        analyte = UrineAnalyte.Leukocytes,
        name = "Leukocytes",
        swatches = listOf(
            Color(0xFFF5F0DC),
            Color(0xFFFFCCDD),
            Color(0xFFDD88BB),
            Color(0xFF995599),
        ),
        defaultIndex = null,
        valueLabels = listOf("Negative", "Trace", "Mild", "High"),
    ),
)

/**
 * Urine strip log — Pencil `N00Rs.png`, spec §4.10.
 *
 * Renders the four analytes (Glucose, pH, Protein, Leukocytes) as colour
 * swatch rows. Tapping a swatch selects it (2 dp `ohd-ink` outside stroke);
 * the right-aligned value label updates from the analyte spec. Selections
 * are hoisted via [onLog].
 */
@Composable
fun UrineStripScreen(
    onBack: () -> Unit,
    onLog: (Map<UrineAnalyte, Int?>) -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
) {
    var selections by remember {
        mutableStateOf(Analytes.associate { it.analyte to it.defaultIndex })
    }

    // Persist + forward to the caller. Earlier versions of this screen
    // hoisted the persistence to NavGraph which only toasted a fake-success;
    // doing the `putEvent` here means the row actually lands in Recent
    // Events. Event type is registered by migration 018 as
    // `measurement.urine_strip` with one text channel per analyte (Glucose
    // / pH / Protein / Leukocytes) using the analyte's value label
    // ("Negative", "7.0", "Trace", …) rather than the swatch index.
    fun persistAndForward() {
        val channels = selections.mapNotNull { (analyte, idx) ->
            if (idx == null) return@mapNotNull null
            val spec = Analytes.first { it.analyte == analyte }
            val label = spec.valueLabels.getOrNull(idx) ?: return@mapNotNull null
            EventChannelInput(
                path = analyte.name.lowercase(),
                scalar = OhdScalar.Text(label),
            )
        }
        StorageRepository.putEvent(
            EventInput(
                timestampMs = System.currentTimeMillis(),
                eventType = "measurement.urine_strip",
                channels = channels,
            ),
        )
        onLog(selections)
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(
            title = "Urine Strip",
            onBack = onBack,
            action = TopBarAction(label = "Log", onClick = { persistAndForward() }),
        )

        // Notice strip.
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .background(OhdColors.BgElevated)
                .padding(horizontal = 16.dp, vertical = 10.dp),
        ) {
            Text(
                text = "Pick the colour closest to your strip. Tap to select.",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
        }

        LazyColumn(modifier = Modifier.fillMaxWidth()) {
            items(Analytes) { spec ->
                val selected = selections[spec.analyte]
                UrineAnalyteRow(
                    spec = spec,
                    selectedIndex = selected,
                    onSelectIndex = { idx ->
                        selections = selections.toMutableMap().apply { put(spec.analyte, idx) }
                    },
                )
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(1.dp)
                        .background(OhdColors.Line),
                )
            }
        }
    }
}

@Composable
private fun UrineAnalyteRow(
    spec: AnalyteSpec,
    selectedIndex: Int?,
    onSelectIndex: (Int) -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        // Header row: name + value.
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = spec.name,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 14.sp,
                color = OhdColors.Ink,
                modifier = Modifier.weight(1f),
            )
            val valueText = selectedIndex?.let { spec.valueLabels.getOrNull(it) } ?: "—"
            // pH renders the healthy value (7.0) in success green.
            val valueColor = if (
                spec.healthyValueIndex != null &&
                selectedIndex == spec.healthyValueIndex
            ) {
                OhdColors.Success
            } else {
                OhdColors.Muted
            }
            Text(
                text = valueText,
                fontFamily = OhdMono,
                fontWeight = FontWeight.W400,
                fontSize = 13.sp,
                color = valueColor,
            )
        }

        // Swatches row.
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            spec.swatches.forEachIndexed { idx, color ->
                Swatch(
                    color = color,
                    selected = selectedIndex == idx,
                    onClick = { onSelectIndex(idx) },
                    modifier = Modifier.weight(1f),
                )
            }
        }

        // Optional caption row.
        if (spec.captionLow != null || spec.captionHigh != null) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = spec.captionLow.orEmpty(),
                    fontFamily = OhdMono,
                    fontWeight = FontWeight.W400,
                    fontSize = 10.sp,
                    color = OhdColors.Muted,
                )
                Text(
                    text = spec.captionHigh.orEmpty(),
                    fontFamily = OhdMono,
                    fontWeight = FontWeight.W400,
                    fontSize = 10.sp,
                    color = OhdColors.Muted,
                )
            }
        }
    }
}

@Composable
private fun Swatch(
    color: Color,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val shape = RoundedCornerShape(4.dp)
    val borderModifier = if (selected) {
        Modifier.border(BorderStroke(2.dp, OhdColors.Ink), shape)
    } else {
        Modifier
    }
    Box(
        modifier = modifier
            .height(36.dp)
            .background(color, shape)
            .then(borderModifier)
            .clickable { onClick() },
    )
}

