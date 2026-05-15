package com.ohd.connect.ui.screens

import androidx.compose.ui.graphics.vector.ImageVector
import com.ohd.connect.ui.components.TakenState
import com.ohd.connect.ui.icons.OhdIcons

/**
 * Stub literals for v1 screens that don't yet have a real data source.
 *
 * Centralised so they're trivial to delete once the corresponding repository
 * methods land. Anything in here is **not** persisted — it's UI scaffolding
 * for the visual rebuild. See spec §7.
 */
internal object StubData {
    // -------------------------------------------------------------------------
    // Home — favourites strip (KADlx)
    // -------------------------------------------------------------------------
    data class Favourite(val label: String, val icon: ImageVector)

    val homeFavourites: List<Favourite> = listOf(
        Favourite("Glucose", OhdIcons.Droplets),
        Favourite("Blood pressure", OhdIcons.HeartPulse),
    )

    // -------------------------------------------------------------------------
    // Medication — prescribed + on-hand (LURIu)
    //
    // The legacy `MedRow` shape (name + sub + pre-baked TakenState) is kept
    // as a convenience for any test/preview that wants the static look-alike.
    // The screen itself works off the richer `Medication` model below, which
    // carries the dose / unit / schedule needed to wire real persistence and
    // compute "missed" / "taken" dynamically from recent events.
    // -------------------------------------------------------------------------
    data class MedRow(val name: String, val sub: String, val state: TakenState)

    enum class MedKind { Prescribed, OnHand }

    /**
     * Source-of-truth row for the medication screen.
     *
     * @param defaultDose Numeric dose prefilled in the long-press dialog and
     *   used for short-press logging. Real value (mg/mL/IU) per [unit].
     * @param scheduleHours For prescribed meds, how often a dose is due. The
     *   screen treats `now - lastTakenAt < scheduleHours` as `Taken`,
     *   otherwise `Pending` → counted as "missed". On-hand meds default
     *   `null` (never missed).
     */
    data class Medication(
        val name: String,
        val sub: String,
        val defaultDose: Double,
        val unit: String,
        val kind: MedKind,
        val scheduleHours: Int? = null,
    )

    val medications: List<Medication> = listOf(
        Medication(
            name = "Metformin 500 mg",
            sub = "Prescribed · twice daily · due now",
            defaultDose = 500.0,
            unit = "mg",
            kind = MedKind.Prescribed,
            scheduleHours = 12,
        ),
        Medication(
            name = "Lisinopril 10 mg",
            sub = "Prescribed · once daily",
            defaultDose = 10.0,
            unit = "mg",
            kind = MedKind.Prescribed,
            scheduleHours = 24,
        ),
        Medication(
            name = "Vitamin D3 2000 IU",
            sub = "On-hand",
            defaultDose = 2000.0,
            unit = "IU",
            kind = MedKind.OnHand,
            scheduleHours = 24,
        ),
        Medication(
            name = "Omega-3 1000 mg",
            sub = "On-hand",
            defaultDose = 1000.0,
            unit = "mg",
            kind = MedKind.OnHand,
            scheduleHours = 24,
        ),
    )

    /** Legacy stub list — kept for any consumer still on `MedRow`. */
    val prescribed: List<MedRow> = listOf(
        MedRow("Metformin 500 mg", "Prescribed · twice daily · due now", TakenState.Pending),
        MedRow("Lisinopril 10 mg", "Prescribed · once daily · taken 8h ago", TakenState.Taken),
    )

    val onHand: List<MedRow> = listOf(
        MedRow("Vitamin D3 2000 IU", "Last taken · 3 days ago", TakenState.Taken),
        MedRow("Omega-3 1000 mg", "Last taken · today", TakenState.Taken),
    )

    val prescribedMissedCount: Int = 1

    // -------------------------------------------------------------------------
    // Recent events fallback list (H06Ms) — used when audit_query is empty
    // or storage isn't open. Each entry is a (primary, meta) tuple.
    // -------------------------------------------------------------------------
    data class RecentEntry(val primary: String, val meta: String)

    val recentEventsSample: List<RecentEntry> = listOf(
        RecentEntry("Medication · Metformin 500 mg", "Today 09:14"),
        RecentEntry("Measurement · Glucose 5.4 mmol/L", "Today 08:47"),
        RecentEntry("Food · Oat porridge with banana", "Today 08:05"),
        RecentEntry("Symptom · Fatigue 2/5", "Yesterday 22:30"),
        RecentEntry("Medication · Lisinopril 10 mg", "Yesterday 21:00"),
        RecentEntry("Measurement · Blood pressure 118/76", "Yesterday 08:30"),
    )
}
