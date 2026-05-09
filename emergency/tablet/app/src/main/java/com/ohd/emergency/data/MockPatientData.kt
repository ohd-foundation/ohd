package com.ohd.emergency.data

/**
 * Hand-rolled patient dataset for the v0 demo flow.
 *
 * Designed to look like a *credible* mid-50s patient with one chronic
 * condition (HFrEF on bisoprolol + furosemide) and a couple of recent
 * vital readings, so that the patient-view screen looks lived-in
 * during a demo. Not a real person; the values are within typical
 * clinical ranges for someone presenting with mild decompensation.
 *
 * Structured this way (instead of a single fat record) so individual
 * screens can borrow components without dragging in the whole bundle:
 *   - PatientView screen calls [exampleView]
 *   - Timeline screen calls [exampleTimeline]
 *   - VitalsPad components call [recentSeriesFor]
 *
 * Replaced wholesale once the OHDC HTTP client is wired and
 * `EmergencyRepository.loadPatientView` issues a real
 * `OhdcService.QueryEvents` against the case grant.
 */
internal object MockPatientData {

    private const val ONE_HOUR_MS = 3600 * 1000L
    private const val ONE_MINUTE_MS = 60 * 1000L

    fun exampleView(caseUlid: String): PatientView {
        val now = System.currentTimeMillis()
        return PatientView(
            caseUlid = caseUlid,
            patientLabel = "Patient (label withheld)",
            patientAge = 58,
            patientSex = "M",
            openedAtMs = now - 12 * ONE_MINUTE_MS,
            criticalInfo = CriticalInfo(
                allergies = listOf(
                    "Penicillin (rash, 2014)",
                    "Contrast dye (mild reaction, 2019)",
                ),
                bloodType = "A+",
                advanceDirectives = listOf(
                    "Do-not-resuscitate (DNR) — on file 2024-03-12",
                    "No artificial nutrition / hydration in terminal stage",
                ),
                flagsAtAGlance = listOf(
                    "Anticoagulant: warfarin",
                    "Pacemaker (Medtronic, 2022)",
                ),
            ),
            activeMedications = listOf(
                MedicationEntry("Bisoprolol", "5 mg PO q24h", now - 7 * ONE_HOUR_MS),
                MedicationEntry("Furosemide", "40 mg PO q24h", now - 8 * ONE_HOUR_MS),
                MedicationEntry("Warfarin", "3 mg PO q24h", now - 18 * ONE_HOUR_MS),
                MedicationEntry("Ramipril", "5 mg PO q24h", now - 7 * ONE_HOUR_MS),
            ),
            recentVitals = listOf(
                VitalSnapshot(
                    channel = "vital.hr",
                    displayLabel = "Heart rate",
                    latestValue = "112",
                    latestUnit = "bpm",
                    takenAtMs = now - 2 * ONE_MINUTE_MS,
                    series = recentSeriesFor("vital.hr"),
                ),
                VitalSnapshot(
                    channel = "vital.bp",
                    displayLabel = "Blood pressure",
                    latestValue = "92/56",
                    latestUnit = "mmHg",
                    takenAtMs = now - 2 * ONE_MINUTE_MS,
                    series = recentSeriesFor("vital.bp_sys"),
                ),
                VitalSnapshot(
                    channel = "vital.spo2",
                    displayLabel = "SpO2",
                    latestValue = "91",
                    latestUnit = "%",
                    takenAtMs = now - 90 * 1000L,
                    series = recentSeriesFor("vital.spo2"),
                ),
                VitalSnapshot(
                    channel = "vital.temp",
                    displayLabel = "Temp",
                    latestValue = "37.4",
                    latestUnit = "°C",
                    takenAtMs = now - 4 * ONE_MINUTE_MS,
                    series = recentSeriesFor("vital.temp"),
                ),
                VitalSnapshot(
                    channel = "vital.gcs",
                    displayLabel = "GCS",
                    latestValue = "14",
                    latestUnit = "/15",
                    takenAtMs = now - 3 * ONE_MINUTE_MS,
                    series = recentSeriesFor("vital.gcs"),
                ),
            ),
            activeDiagnoses = listOf(
                "Heart failure with reduced ejection fraction (HFrEF, EF 35%)",
                "Atrial fibrillation, paroxysmal",
                "Type 2 diabetes mellitus (HbA1c 7.2)",
                "Chronic kidney disease, stage 3a",
            ),
            recentObservations = listOf(
                ObservationEntry(
                    timestampMs = now - 6 * ONE_HOUR_MS,
                    text = "Cardio follow-up: stable on current regimen, EF unchanged.",
                ),
                ObservationEntry(
                    timestampMs = now - 3 * 24 * ONE_HOUR_MS,
                    text = "GP visit: increased peripheral oedema, weight +1.5 kg over 4 days.",
                ),
            ),
        )
    }

    /** Mock recent timeline — events the case opened with + initial response. */
    fun exampleTimeline(caseUlid: String): List<TimelineEntry> {
        val now = System.currentTimeMillis()
        return listOf(
            TimelineEntry(
                ulid = "01EVTSTART",
                timestampMs = now - 12 * ONE_MINUTE_MS,
                kind = TimelineKind.GrantOpened,
                summary = "Break-glass auto-granted (timeout) — emergency template applied",
            ),
            TimelineEntry(
                ulid = "01EVTOBS01",
                timestampMs = now - 10 * ONE_MINUTE_MS,
                kind = TimelineKind.Observation,
                summary = "Chief complaint: shortness of breath, ankle swelling",
            ),
            TimelineEntry(
                ulid = "01EVTHR01",
                timestampMs = now - 8 * ONE_MINUTE_MS,
                kind = TimelineKind.Vital,
                summary = "HR 118 bpm, BP 90/55, SpO2 90%",
            ),
            TimelineEntry(
                ulid = "01EVTDRG01",
                timestampMs = now - 6 * ONE_MINUTE_MS,
                kind = TimelineKind.Drug,
                summary = "O2 4 L/min via nasal cannula",
            ),
            TimelineEntry(
                ulid = "01EVTHR02",
                timestampMs = now - 3 * ONE_MINUTE_MS,
                kind = TimelineKind.Vital,
                summary = "HR 112 bpm, SpO2 92%",
            ),
        )
    }

    /**
     * Mock vitals series — last ~30 minutes of values for a given channel.
     * Used by the sparkline component on the patient view.
     */
    fun recentSeriesFor(channel: String): List<VitalReading> {
        val now = System.currentTimeMillis()
        // 30-minute window, one reading every 3 minutes ⇒ 10 points.
        // Channel-specific shape so the sparklines look distinct.
        return List(10) { i ->
            val t = now - (10 - i) * 3 * ONE_MINUTE_MS
            val value = when (channel) {
                "vital.hr" -> 122.0 - i * 0.9 + (if (i % 3 == 0) 1.5 else -1.0)
                "vital.spo2" -> 88.5 + i * 0.25
                "vital.bp_sys" -> 96.0 - i * 0.4
                "vital.temp" -> 37.6 - i * 0.02
                "vital.gcs" -> if (i < 4) 13.0 else 14.0
                else -> 50.0 + i.toDouble()
            }
            VitalReading(timestampMs = t, value = value)
        }
    }
}
