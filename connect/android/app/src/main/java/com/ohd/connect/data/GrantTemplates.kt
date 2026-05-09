package com.ohd.connect.data

/**
 * Grant-creation templates surfaced by the Grants tab.
 *
 * Mirrors `connect/web/src/ohdc/store.ts` `GRANT_TEMPLATES` so Android and
 * web produce byte-identical default scopes (the user can adjust on either
 * front-end and the resulting grant looks the same to the storage backend).
 *
 * Per `connect/SPEC.md` "Grant management UX → Templates":
 *   - primary_doctor: all channels, 1y, auto-approve labs/notes
 *   - specialist_visit: all channels, 30d, every write needs approval
 *   - spouse_family: vitals + emergency profile, indefinite, read-only
 *   - researcher: aggregation only, strip notes, study window
 *   - emergency_break_glass: emergency-template (cloned on incident)
 */
object GrantTemplates {

    enum class Id(val key: String, val label: String, val sub: String) {
        PRIMARY_DOCTOR(
            key = "primary_doctor",
            label = "Primary doctor",
            sub = "All channels, 1-year expiry, auto-approve labs/notes.",
        ),
        SPECIALIST_VISIT(
            key = "specialist_visit",
            label = "Specialist for one visit",
            sub = "30-day scope, every write needs your approval.",
        ),
        SPOUSE_FAMILY(
            key = "spouse_family",
            label = "Spouse / family",
            sub = "Read-only, vitals + emergency profile, indefinite.",
        ),
        RESEARCHER(
            key = "researcher",
            label = "Researcher with study",
            sub = "Aggregation only, strip notes, study window.",
        ),
        EMERGENCY_BREAK_GLASS(
            key = "emergency_break_glass",
            label = "Emergency break-glass",
            sub = "Template for first responders. Cloned on incident.",
        ),
    }

    private val ALL_READ = listOf(
        "std.blood_glucose",
        "std.heart_rate_resting",
        "std.body_temperature",
        "std.blood_pressure",
        "std.medication_dose",
        "std.symptom",
        "std.meal",
        "std.mood",
        "std.clinical_note",
    )

    private val SENSITIVE_OPTIONAL = listOf(
        "mental_health",
        "substance_use",
        "sexual_health",
        "reproductive",
    )

    /** Construct a [CreateGrantInput] pre-populated for [id]. */
    fun forTemplate(id: Id, granteeLabel: String, purpose: String? = null): CreateGrantInput {
        val nowMs = System.currentTimeMillis()
        return when (id) {
            Id.PRIMARY_DOCTOR -> CreateGrantInput(
                granteeLabel = granteeLabel,
                granteeKind = "user",
                purpose = purpose,
                approvalMode = "auto_for_event_types",
                defaultAction = "allow",
                expiresAtMs = nowMs + 365L * 86_400_000L,
                readEventTypes = ALL_READ,
                writeEventTypes = listOf("std.clinical_note", "std.medication_dose"),
                autoApproveEventTypes = listOf("std.clinical_note"),
                denySensitivityClasses = SENSITIVE_OPTIONAL,
                notifyOnAccess = false,
            )
            Id.SPECIALIST_VISIT -> CreateGrantInput(
                granteeLabel = granteeLabel,
                granteeKind = "user",
                purpose = purpose,
                approvalMode = "always",
                defaultAction = "allow",
                expiresAtMs = nowMs + 30L * 86_400_000L,
                readEventTypes = ALL_READ,
                writeEventTypes = listOf("std.clinical_note"),
                denySensitivityClasses = SENSITIVE_OPTIONAL,
                notifyOnAccess = false,
            )
            Id.SPOUSE_FAMILY -> CreateGrantInput(
                granteeLabel = granteeLabel,
                granteeKind = "user",
                purpose = purpose,
                approvalMode = "always",
                defaultAction = "allow",
                expiresAtMs = null,
                readEventTypes = listOf(
                    "std.heart_rate_resting",
                    "std.body_temperature",
                    "std.blood_pressure",
                    "std.blood_glucose",
                ),
                writeEventTypes = emptyList(),
                denySensitivityClasses = SENSITIVE_OPTIONAL,
                notifyOnAccess = true,
            )
            Id.RESEARCHER -> CreateGrantInput(
                granteeLabel = granteeLabel,
                granteeKind = "researcher",
                purpose = purpose,
                approvalMode = "always",
                defaultAction = "allow",
                expiresAtMs = nowMs + 90L * 86_400_000L,
                readEventTypes = ALL_READ,
                writeEventTypes = emptyList(),
                denySensitivityClasses = SENSITIVE_OPTIONAL,
                notifyOnAccess = true,
                stripNotes = true,
                aggregationOnly = true,
            )
            Id.EMERGENCY_BREAK_GLASS -> CreateGrantInput(
                granteeLabel = granteeLabel,
                granteeKind = "emergency_authority",
                purpose = purpose,
                approvalMode = "auto_for_event_types",
                defaultAction = "allow",
                expiresAtMs = nowMs + 1L * 86_400_000L,
                readEventTypes = listOf(
                    "std.blood_glucose",
                    "std.heart_rate_resting",
                    "std.body_temperature",
                    "std.blood_pressure",
                    "std.medication_dose",
                ),
                writeEventTypes = emptyList(),
                denySensitivityClasses = SENSITIVE_OPTIONAL,
                notifyOnAccess = true,
            )
        }
    }
}
