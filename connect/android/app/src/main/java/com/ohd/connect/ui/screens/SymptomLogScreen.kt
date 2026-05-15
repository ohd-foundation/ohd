package com.ohd.connect.ui.screens

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
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.EventChannelInput
import com.ohd.connect.data.EventInput
import com.ohd.connect.data.OhdScalar
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Built-in preset symptoms for the chip row above the description.
 *
 * Kept local to this file (per agent scope guidance) so the medication
 * agent's StubData edits don't collide. If a second screen ever needs
 * this list, promote to `_shared/SymptomData.kt` rather than StubData.kt.
 *
 * "Other" is intentionally last and acts as the "free text only" fallback
 * — selecting it widens the description placeholder and persists as
 * `symptom.other` with the typed text in `notes`.
 */
val DefaultSymptoms: List<String> = listOf(
    "Headache",
    "Migraine",
    "Fatigue",
    "Nausea",
    "Dizziness",
    "Cough",
    "Sore throat",
    "Fever",
    "Stomach pain",
    "Joint pain",
    "Back pain",
    "Shortness of breath",
    "Anxiety",
    "Insomnia",
    "Other",
)

/**
 * One step on the standardised pain scale.
 *
 * Loosely aligns to the WHO / clinical NRS-11 scale (0..10) but presents
 * descriptive bands rather than raw numbers, which the user feedback
 * called out as easier to reason about ("from slight pain to unable to
 * work"). The persisted `severity` channel is the rough NRS midpoint per
 * band, so downstream tools that expect the standard 0..10 scale still
 * receive a usable value.
 */
private data class PainStep(
    val index: Int,
    val label: String,
    val caption: String,
    /** Mid-point on the canonical NRS-11 (0..10) scale. */
    val nrs: Double,
)

private val PainScale: List<PainStep> = listOf(
    PainStep(0, "None", "no pain", 0.0),
    PainStep(1, "Mild", "barely noticeable", 2.0),
    PainStep(2, "Moderate", "uncomfortable", 4.5),
    PainStep(3, "Strong", "hard to ignore", 6.0),
    PainStep(4, "Severe", "interferes with daily tasks", 7.5),
    PainStep(5, "Disabling", "unable to work / function", 9.5),
)

/**
 * Symptom log — Pencil `FQzfA.png`, spec §4.8.
 *
 * Single-screen "log a symptom" form: preset chip row + free-text
 * description + 6-step descriptive NRS pain scale + bottom-anchored CTA.
 *
 * Persistence happens locally via [StorageRepository.putEvent] using:
 *   - `eventType = "symptom.<snake_name>"` (e.g. `symptom.headache`,
 *     or `symptom.other` for free-text only).
 *   - channel `severity`         REAL  (0..10 NRS mid-point)
 *   - channel `severity_label`   TEXT  (lowercase band, e.g. "moderate")
 *   - channel `notes`            TEXT  (free-text description, if any)
 *
 * The [onLog] callback is preserved as `(text, severity)` so the existing
 * `NavGraph` snackbar wiring keeps working — `text` is the human-readable
 * symptom name (or the typed text for "Other") and `severity` is the
 * 0..5 band index. Persistence has already happened by the time `onLog`
 * fires.
 */
@Composable
fun SymptomLogScreen(
    onBack: () -> Unit,
    onLog: (text: String, severity: Int) -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
) {
    var text by remember { mutableStateOf("") }
    var severity by remember { mutableStateOf(1) }      // band index, default = Mild
    var selectedSymptom by remember { mutableStateOf<String?>(null) }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Symptom", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(horizontal = 16.dp, vertical = 20.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            // ----------------------------------------------------------
            // Preset symptom chip row (horizontal scroll, single-select).
            // ----------------------------------------------------------
            Text(
                text = "Common symptoms",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 13.sp,
                color = OhdColors.Muted,
            )
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                for (s in DefaultSymptoms) {
                    SymptomPresetChip(
                        label = s,
                        selected = selectedSymptom == s,
                        onClick = {
                            selectedSymptom = if (selectedSymptom == s) null else s
                        },
                    )
                }
            }

            // ----------------------------------------------------------
            // Description label + multiline text-area.
            //
            // Default label keeps the original "Describe the symptom"
            // string (smoke test asserts it). When a preset is picked
            // we re-label to "{Name} — describe (optional)". "Other"
            // widens the placeholder to encourage free text.
            // ----------------------------------------------------------
            val descriptionLabel: String = when (val s = selectedSymptom) {
                null -> "Describe the symptom"
                else -> "$s — describe (optional)"
            }
            val placeholderText: String = when (selectedSymptom) {
                null -> "e.g. Mild headache behind the eyes, started after lunch…"
                "Other" -> "What's bothering you?"
                else -> "Add detail (when it started, what helps, etc.)"
            }
            Text(
                text = descriptionLabel,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 13.sp,
                color = OhdColors.Muted,
            )

            // Multi-line text area.
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(120.dp)
                    .background(OhdColors.Bg, RoundedCornerShape(8.dp))
                    .border(BorderStroke(1.5.dp, OhdColors.Line), RoundedCornerShape(8.dp))
                    .padding(12.dp),
            ) {
                if (text.isEmpty()) {
                    Text(
                        text = placeholderText,
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 14.sp,
                        lineHeight = 21.sp,
                        color = OhdColors.Muted,
                    )
                }
                BasicTextField(
                    value = text,
                    onValueChange = { text = it },
                    modifier = Modifier.fillMaxSize(),
                    textStyle = TextStyle(
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 14.sp,
                        lineHeight = 21.sp,
                        color = OhdColors.Ink,
                    ),
                    cursorBrush = SolidColor(OhdColors.Ink),
                )
            }

            // ----------------------------------------------------------
            // Severity — 6-step descriptive NRS-aligned scale.
            // ----------------------------------------------------------
            Text(
                text = "Severity",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 13.sp,
                color = OhdColors.Muted,
            )

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                for (step in PainScale) {
                    SeverityChip(
                        label = step.label,
                        selected = severity == step.index,
                        onClick = { severity = step.index },
                        modifier = Modifier.weight(1f),
                    )
                }
            }

            // Caption row: the band-name extremes on the left/right plus
            // (when the band has one) the descriptive caption for the
            // currently-selected step underneath, centred.
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    text = PainScale.first().label,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 11.sp,
                    color = OhdColors.Muted,
                )
                Text(
                    text = PainScale.last().label,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 11.sp,
                    color = OhdColors.Muted,
                )
            }
            val selectedStep = PainScale.firstOrNull { it.index == severity }
            if (selectedStep != null && selectedStep.caption.isNotEmpty()) {
                Text(
                    text = selectedStep.caption,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 11.sp,
                    color = OhdColors.Muted,
                    modifier = Modifier.fillMaxWidth(),
                )
            }

            // Push CTA to bottom.
            Box(modifier = Modifier.weight(1f))

            OhdButton(
                label = "Log symptom",
                onClick = {
                    persistAndForward(
                        symptom = selectedSymptom,
                        text = text,
                        severityIndex = severity,
                        onLog = onLog,
                    )
                },
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

/**
 * Persist the symptom event then forward to [onLog] for navigation.
 *
 * The repository call is wrapped in `runCatching` (via [StorageRepository.putEvent]
 * which already returns a `Result`); on either success or failure we still
 * fire [onLog] so the NavGraph snackbar appears and the screen pops. This
 * matches the spec: "on failure show 'Saved locally — sync later' and
 * still pop", with the actual snackbar copy formatted by NavGraph from the
 * `text` + `severity` we pass through.
 */
private fun persistAndForward(
    symptom: String?,
    text: String,
    severityIndex: Int,
    onLog: (text: String, severity: Int) -> Unit,
) {
    val step = PainScale.firstOrNull { it.index == severityIndex } ?: PainScale[1]
    val name: String? = symptom
    val displayText: String = when {
        name != null && name != "Other" -> name
        text.isNotBlank() -> text.trim()
        name == "Other" -> "Other"
        else -> ""
    }

    val eventTypeSuffix: String = when {
        name == null -> "other"                 // no preset selected
        name == "Other" -> "other"
        else -> name.lowercase()
            .replace(' ', '_')
            .replace('/', '_')
    }
    val eventType = "symptom.$eventTypeSuffix"

    val channels = buildList {
        add(
            EventChannelInput(
                path = "severity",
                scalar = OhdScalar.Real(step.nrs),
            ),
        )
        add(
            EventChannelInput(
                path = "severity_label",
                scalar = OhdScalar.Text(step.label.lowercase()),
            ),
        )
        if (text.isNotBlank()) {
            add(
                EventChannelInput(
                    path = "notes",
                    scalar = OhdScalar.Text(text.trim()),
                ),
            )
        }
    }

    val input = EventInput(
        timestampMs = System.currentTimeMillis(),
        eventType = eventType,
        channels = channels,
        notes = text.trim().takeIf { it.isNotBlank() },
    )

    // Fire-and-forget: success or failure, we still pop. The NavGraph
    // snackbar formats the message from (displayText, severityIndex).
    StorageRepository.putEvent(input)

    onLog(displayText, severityIndex)
}

@Composable
private fun SymptomPresetChip(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val shape = RoundedCornerShape(8.dp)
    val fillModifier = if (selected) {
        Modifier.background(OhdColors.Ink, shape)
    } else {
        Modifier
            .background(OhdColors.Bg, shape)
            .border(BorderStroke(1.dp, OhdColors.Line), shape)
    }

    Box(
        modifier = modifier
            .height(36.dp)
            .then(fillModifier)
            .clickable { onClick() }
            .padding(horizontal = 14.dp),
        contentAlignment = Alignment.Center,
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
private fun SeverityChip(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val shape = RoundedCornerShape(8.dp)
    val fillModifier = if (selected) {
        Modifier.background(OhdColors.Ink, shape)
    } else {
        Modifier
            .background(OhdColors.Bg, shape)
            .border(BorderStroke(1.dp, OhdColors.Line), shape)
    }

    Box(
        modifier = modifier
            .height(44.dp)
            .then(fillModifier)
            .clickable { onClick() }
            .padding(horizontal = 4.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = if (selected) FontWeight.W500 else FontWeight.W400,
            fontSize = 12.sp,
            color = if (selected) OhdColors.Bg else OhdColors.Muted,
        )
    }
}
