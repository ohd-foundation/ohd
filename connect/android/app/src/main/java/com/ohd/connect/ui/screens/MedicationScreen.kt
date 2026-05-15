package com.ohd.connect.ui.screens

import androidx.compose.foundation.background
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.EventChannelInput
import com.ohd.connect.data.EventFilter
import com.ohd.connect.data.EventInput
import com.ohd.connect.data.OhdScalar
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdMedLogItem
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TakenState
import com.ohd.connect.ui.components.TopBarAction
import com.ohd.connect.ui.screens._shared.AddOnHandDialog
import com.ohd.connect.ui.screens._shared.TakeMedDialog
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Medication v2 — Pencil `LURIu`, spec §4.2.
 *
 * Top bar (back + "Medications" + "Library" action) over PRESCRIBED and ON-HAND
 * sections, then a footer with a Ghost "+ Add to on-hand" button.
 *
 * State model:
 *  - The medication catalogue lives in [StubData.medications] — name, default
 *    dose/unit, prescribed/on-hand kind, schedule cadence in hours.
 *  - `lastTakenAt` is a per-medication-name map of the most recent
 *    `medication.taken` event timestamp. Backed by [StorageRepository.queryEvents]
 *    on first composition, and updated optimistically when the user logs a
 *    dose so the UI feels instant.
 *  - For each prescribed med we compute `Taken` if `now - lastTakenAt < scheduleHours`,
 *    otherwise `Pending` → contributes to the red "X missed" badge.
 *
 * Persistence — short-press logs `now` + the medication's default dose; long
 * press opens [TakeMedDialog] which lets the user override time / dose / unit.
 * Both routes call `StorageRepository.putEvent` with `eventType =
 * "medication.taken"`. See `medicationTakenInput` for the exact channel layout.
 */
@Composable
fun MedicationScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onOpenLibrary: () -> Unit,
    onLogMedication: (String) -> Unit = {},
    onToast: (String) -> Unit = {},
) {
    // Last-taken timestamp per medication name. Populated from storage on
    // first composition; mutated optimistically on each tap so the UI updates
    // immediately even before the storage write returns.
    val lastTakenAt = remember { mutableStateMapOf<String, Long>() }

    // The medication currently being long-pressed. Non-null while the
    // TakeMedDialog is visible.
    var dialogTarget by remember { mutableStateOf<StubData.Medication?>(null) }

    // True while the "+ Add to on-hand" dialog is open. We host the dialog
    // here (rather than via a nav callback) because the captured form
    // state lives entirely inside the dialog composable.
    var addOnHandOpen by remember { mutableStateOf(false) }

    // Preload existing `medication.taken` events on first compose. Works
    // when storage is open — falls back to an empty map (no rows show as
    // recently taken) when it isn't, which matches the brief's contract:
    // wrap in runCatching so storage-not-open during a test run is a no-op.
    LaunchedEffect(Unit) {
        runCatching {
            StorageRepository.queryEvents(
                EventFilter(
                    eventTypesIn = listOf(MEDICATION_TAKEN_EVENT_TYPE),
                    limit = 200L,
                ),
            )
        }.getOrNull()?.getOrNull()?.forEach { event ->
            // Pull the medication name out of the channels. The writer
            // embeds it in the `med.name` text channel; older rows that
            // landed via the legacy `std.medication_dose` path have a `name`
            // channel — accept either so we don't black-hole history.
            val name = event.channels
                .firstOrNull { it.path == CHANNEL_NAME || it.path == "name" }
                ?.display
                ?: return@forEach
            val current = lastTakenAt[name]
            if (current == null || event.timestampMs > current) {
                lastTakenAt[name] = event.timestampMs
            }
        }
    }

    // Helper: persist a `medication.taken` event and mirror into the local
    // map. Storage failures (e.g. handle not open during tests) are
    // swallowed — the optimistic UI update survives.
    fun logTake(med: StubData.Medication, timestampMs: Long, dose: Double, unit: String) {
        lastTakenAt[med.name] = timestampMs
        runCatching {
            StorageRepository.putEvent(
                medicationTakenInput(
                    timestampMs = timestampMs,
                    name = med.name,
                    dose = dose,
                    unit = unit,
                ),
            )
        }
        onLogMedication(med.name)
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(
            title = "Medications",
            onBack = onBack,
            action = TopBarAction(label = "Library", onClick = onOpenLibrary),
        )

        val prescribed = StubData.medications.filter { it.kind == StubData.MedKind.Prescribed }
        val onHand = StubData.medications.filter { it.kind == StubData.MedKind.OnHand }

        // Recompute "missed" each composition — drops as soon as the user
        // taps Take on a Pending prescribed row.
        val missedCount = prescribed.count { med ->
            takenStateFor(med, lastTakenAt[med.name]) == TakenState.Pending
        }

        // 1. PRESCRIBED header with dynamic missed badge.
        PrescribedHeader(missedCount = missedCount)

        // 2. Prescribed rows.
        prescribed.forEachIndexed { idx, med ->
            val ts = lastTakenAt[med.name]
            val state = takenStateFor(med, ts)
            OhdMedLogItem(
                name = med.name,
                sub = subtitleFor(med, ts),
                takenState = state,
                onLog = { logTake(med, System.currentTimeMillis(), med.defaultDose, med.unit) },
                onLongPress = { dialogTarget = med },
            )
            if (idx < prescribed.lastIndex) OhdDivider()
        }

        // 3. ON-HAND header.
        Box(modifier = Modifier.padding(top = 14.dp, bottom = 8.dp)) {
            // Section header has its own [v=8 h=16] padding; the wrapper Box
            // adds the spec's `t=14, b=8` *extra* top padding before the
            // header so the on-hand block reads with breathing room above.
            Text(
                text = "ON-HAND",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 11.sp,
                letterSpacing = 2.sp,
                color = OhdColors.Muted,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp),
            )
        }

        // 4. On-hand rows.
        onHand.forEachIndexed { idx, med ->
            val ts = lastTakenAt[med.name]
            val state = takenStateFor(med, ts)
            OhdMedLogItem(
                name = med.name,
                sub = subtitleFor(med, ts),
                takenState = state,
                onLog = { logTake(med, System.currentTimeMillis(), med.defaultDose, med.unit) },
                onLongPress = { dialogTarget = med },
            )
            if (idx < onHand.lastIndex) OhdDivider()
        }

        // 5. Spacer pushes the footer to the bottom of the available space.
        Spacer(modifier = Modifier.weight(1f))

        // 6. Footer with 1dp top border + ghost "+ Add to on-hand" button.
        Footer(onAddToOnHand = { addOnHandOpen = true })
    }

    // 7. Long-press dialog overlay.
    dialogTarget?.let { med ->
        TakeMedDialog(
            medicationName = med.name,
            defaultDose = med.defaultDose,
            defaultUnit = med.unit,
            onDismiss = { dialogTarget = null },
            onTake = { ts, dose, unit ->
                logTake(med, ts, dose, unit)
                dialogTarget = null
            },
        )
    }

    // 8. Add-to-on-hand dialog overlay. Storage isn't wired yet — we
    //    surface a snackbar and dismiss. Persistence lands when the
    //    "user medication list" repo exists.
    if (addOnHandOpen) {
        AddOnHandDialog(
            onDismiss = { addOnHandOpen = false },
            onAdd = { name, _, _ ->
                addOnHandOpen = false
                onToast("Added $name to on-hand")
            },
        )
    }
}

@Composable
private fun PrescribedHeader(missedCount: Int) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = "PRESCRIBED",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 11.sp,
            letterSpacing = 2.sp,
            color = OhdColors.Muted,
        )
        Spacer(Modifier.weight(1f))
        if (missedCount > 0) {
            Box(
                modifier = Modifier
                    .background(OhdColors.RedTint, RoundedCornerShape(20.dp))
                    .padding(horizontal = 8.dp, vertical = 3.dp),
            ) {
                Text(
                    text = "$missedCount missed",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 11.sp,
                    color = OhdColors.Red,
                )
            }
        }
    }
}

@Composable
private fun Footer(onAddToOnHand: () -> Unit) {
    Column(modifier = Modifier.fillMaxWidth()) {
        // Top hairline.
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(OhdColors.Line),
        )
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            contentAlignment = Alignment.Center,
        ) {
            OhdButton(
                label = "+ Add to on-hand",
                onClick = onAddToOnHand,
                variant = OhdButtonVariant.Ghost,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

// =============================================================================
// Helpers
// =============================================================================

/** Compute the row's `Pending` / `Taken` state from the last-taken timestamp. */
internal fun takenStateFor(med: StubData.Medication, lastTakenMs: Long?): TakenState {
    val sched = med.scheduleHours ?: return if (lastTakenMs == null) TakenState.Pending else TakenState.Taken
    if (lastTakenMs == null) return TakenState.Pending
    val now = System.currentTimeMillis()
    val windowMs = sched.toLong() * 60L * 60L * 1000L
    return if (now - lastTakenMs < windowMs) TakenState.Taken else TakenState.Pending
}

/**
 * Build the row's "sub" line. Falls back to the medication's static [sub]
 * when there's no recent take, otherwise renders "Last taken · 2h ago" style
 * via [fmtRelative]. Uses "yesterday" / "X days ago" wording for ranges
 * `fmtRelative` already formats consistently across the app.
 */
internal fun subtitleFor(med: StubData.Medication, lastTakenMs: Long?): String {
    if (lastTakenMs == null) return med.sub
    val rel = fmtRelative(lastTakenMs)
    val nice = when {
        rel.endsWith("s ago") -> "just now"
        rel == "1d ago" -> "yesterday"
        else -> rel
    }
    return when (med.kind) {
        StubData.MedKind.Prescribed -> "Prescribed · last taken $nice"
        StubData.MedKind.OnHand -> "Last taken · $nice"
    }
}

// -----------------------------------------------------------------------------
// EventInput shape — `medication.taken`
// -----------------------------------------------------------------------------
//
// Other agents wiring loggers should match this layout so cross-screen reads
// stay consistent:
//
//   eventType:  "medication.taken"
//   channels:
//     - med.name  (Text)  — display name e.g. "Metformin 500 mg"
//     - med.dose  (Real)  — numeric dose, e.g. 500.0
//     - med.unit  (Text)  — unit string, e.g. "mg" / "mL" / "IU"
//   notes:      "Metformin 500 mg" (human-readable summary)
//
// Uses our own `medication.taken` namespace rather than the more general
// `std.medication_dose` path because we want the screen to filter cleanly
// without competing with structured dose-event imports from Health Connect.
// -----------------------------------------------------------------------------

internal const val MEDICATION_TAKEN_EVENT_TYPE = "medication.taken"
internal const val CHANNEL_NAME = "med.name"
internal const val CHANNEL_DOSE = "med.dose"
internal const val CHANNEL_UNIT = "med.unit"

internal fun medicationTakenInput(
    timestampMs: Long,
    name: String,
    dose: Double,
    unit: String,
): EventInput = EventInput(
    timestampMs = timestampMs,
    eventType = MEDICATION_TAKEN_EVENT_TYPE,
    channels = listOf(
        EventChannelInput(path = CHANNEL_NAME, scalar = OhdScalar.Text(name)),
        EventChannelInput(path = CHANNEL_DOSE, scalar = OhdScalar.Real(dose)),
        EventChannelInput(path = CHANNEL_UNIT, scalar = OhdScalar.Text(unit)),
    ),
    notes = "$name ${formatDoseForNotes(dose)} $unit",
)

private fun formatDoseForNotes(d: Double): String =
    if (d == d.toLong().toDouble()) d.toLong().toString() else d.toString()
