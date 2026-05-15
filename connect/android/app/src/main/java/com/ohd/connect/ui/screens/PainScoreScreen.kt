package com.ohd.connect.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.EventChannelInput
import com.ohd.connect.data.EventInput
import com.ohd.connect.data.OhdScalar
import com.ohd.connect.data.PutEventOutcome
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Pain score logger — Numeric Rating Scale (NRS) 0–10.
 *
 * UI:
 *  - [OhdTopBar] "Pain score" + back arrow.
 *  - "Location" text field — free-form, optional ("lower back", "left knee").
 *  - 11-step NRS row of small numbered chips (0..10).
 *  - Descriptive caption beneath the chip row, derived from the selection
 *    via [nrsLabel] (standard NRS wording).
 *  - "Log pain" primary button — writes a `measurement.pain` event with
 *    channels `location` (text), `severity_nrs` (real), `severity_label` (text).
 *
 * Persistence — uses the same flow as the rest of MeasurementScreen:
 * `StorageRepository.putEvent` returning a [PutEventOutcome] that's
 * surfaced via [onToast] / [onLog] depending on the result.
 */
@Composable
fun PainScoreScreen(
    onBack: () -> Unit,
    onLog: () -> Unit,
    onToast: (String) -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
) {
    var location by remember { mutableStateOf("") }
    var severity by remember { mutableIntStateOf(0) }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Pain score", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f)
                .verticalScroll(rememberScrollState()),
        ) {
            OhdSectionHeader(text = "DETAILS")

            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(20.dp),
            ) {
                OhdField(
                    label = "Location",
                    value = location,
                    onValueChange = { location = it },
                    placeholder = "e.g. lower back, left knee…",
                )

                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Text(
                        text = "Severity",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W500,
                        fontSize = 13.sp,
                        color = OhdColors.Ink,
                    )
                    NrsChipRow(
                        selected = severity,
                        onSelect = { severity = it },
                    )
                    Text(
                        text = nrsLabel(severity),
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 13.sp,
                        color = OhdColors.Muted,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }

            Spacer(Modifier.height(16.dp))
        }

        // Bottom CTA — hairline + primary button at the foot of the screen.
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(OhdColors.Line),
        )
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            OhdButton(
                label = "Log pain",
                onClick = {
                    val input = painEventInput(
                        location = location.trim(),
                        severity = severity,
                    )
                    val outcome = StorageRepository.putEvent(input).getOrElse { e ->
                        PutEventOutcome.Error(
                            code = "INTERNAL",
                            message = e.message ?: e::class.simpleName.orEmpty(),
                        )
                    }
                    when (outcome) {
                        is PutEventOutcome.Committed -> {
                            onToast("Logged pain $severity/10")
                            onLog()
                        }
                        is PutEventOutcome.Pending -> {
                            onToast("Pending review · pain $severity/10")
                            onLog()
                        }
                        is PutEventOutcome.Error -> {
                            onToast("Couldn't log: ${outcome.message}")
                        }
                    }
                },
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

/** 11-step horizontally-laid chip row (0–10) at fixed 32 dp each, gap 4 dp. */
@Composable
private fun NrsChipRow(
    selected: Int,
    onSelect: (Int) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        (0..10).forEach { n ->
            val active = n == selected
            val shape = RoundedCornerShape(8.dp)
            val bg = if (active) OhdColors.Ink else OhdColors.Bg
            val labelColor = if (active) OhdColors.White else OhdColors.Ink

            val base = Modifier
                .weight(1f)
                .height(36.dp)
                .background(bg, shape)
                .clickable { onSelect(n) }
            val finalMod = if (active) base else base.border(1.dp, OhdColors.Line, shape)

            Box(modifier = finalMod, contentAlignment = Alignment.Center) {
                Text(
                    text = n.toString(),
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 13.sp,
                    color = labelColor,
                )
            }
        }
    }
}

// =============================================================================
// NRS helpers
// =============================================================================

/**
 * Standard 11-point NRS bucket labels. Wording mirrors the Wong-Baker FACES
 * captions you see in clinical settings — chosen for being instantly
 * recognisable rather than precision-matching a particular framework.
 */
internal fun nrsLabel(nrs: Int): String = when (nrs) {
    0 -> "No pain"
    1, 2 -> "Mild"
    3, 4 -> "Annoying"
    5, 6 -> "Distracting"
    7, 8 -> "Disabling"
    9 -> "Unbearable"
    10 -> "Worst possible"
    else -> ""
}

// -----------------------------------------------------------------------------
// EventInput shape — `measurement.pain`
// -----------------------------------------------------------------------------
//
//   eventType:  "measurement.pain"
//   channels:
//     - location        (Text)  — free-text body site, blank if unspecified
//     - severity_nrs    (Real)  — 0..10
//     - severity_label  (Text)  — bucket label from `nrsLabel`
//   notes:      "Pain 7/10 · lower back"
//
// Registered in storage/migrations/018_connect_android_types.sql so the
// core's UnknownType check passes.
// -----------------------------------------------------------------------------

internal const val PAIN_EVENT_TYPE = "measurement.pain"

internal fun painEventInput(location: String, severity: Int): EventInput {
    val now = System.currentTimeMillis()
    val locTrim = location.trim()
    val label = nrsLabel(severity)
    val notes = if (locTrim.isEmpty()) "Pain $severity/10" else "Pain $severity/10 · $locTrim"
    return EventInput(
        timestampMs = now,
        eventType = PAIN_EVENT_TYPE,
        channels = listOf(
            EventChannelInput(path = "location", scalar = OhdScalar.Text(locTrim)),
            EventChannelInput(path = "severity_nrs", scalar = OhdScalar.Real(severity.toDouble())),
            EventChannelInput(path = "severity_label", scalar = OhdScalar.Text(label)),
        ),
        notes = notes,
    )
}
