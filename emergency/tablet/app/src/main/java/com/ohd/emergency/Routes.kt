package com.ohd.emergency

/**
 * NavHost route table.
 *
 * Top-level flows:
 *
 *     /login                              — paramedic shift-in (operator OIDC stub)
 *     /discovery                          — BLE scan + manual entry
 *     /break-glass/{beaconId}             — break-glass dialog + status
 *     /patient/{caseUlid}                 — patient view (default tab)
 *     /intervention/{caseUlid}            — quick-entry pads (vitals / drugs / etc.)
 *     /timeline/{caseUlid}                — chronological case feed
 *     /handoff/{caseUlid}                 — receiving-facility selector + confirm
 *
 * Per `ux-design.md` "Nav: Bottom tab bar. 4–5 tabs": Connect uses a
 * bottom-tab pattern, but Emergency uses **a stacked single-flow
 * navigation** because the paramedic moves linearly through one
 * patient at a time. A single bottom action bar (per case) on the
 * patient/intervention/timeline triplet replaces tabs — a paramedic
 * never wants to drop the patient view to "browse settings" mid-call.
 *
 * The handoff route is a top-of-stack modal (it dead-ends the case);
 * after success, NavHost resets back to /discovery.
 */
object Routes {
    const val LOGIN = "login"
    const val DISCOVERY = "discovery"

    private const val BREAK_GLASS_PATTERN = "break-glass/{beaconId}"
    private const val PATIENT_PATTERN = "patient/{caseUlid}"
    private const val INTERVENTION_PATTERN = "intervention/{caseUlid}"
    private const val TIMELINE_PATTERN = "timeline/{caseUlid}"
    private const val HANDOFF_PATTERN = "handoff/{caseUlid}"

    fun breakGlassPattern(): String = BREAK_GLASS_PATTERN
    fun patientPattern(): String = PATIENT_PATTERN
    fun interventionPattern(): String = INTERVENTION_PATTERN
    fun timelinePattern(): String = TIMELINE_PATTERN
    fun handoffPattern(): String = HANDOFF_PATTERN

    /**
     * Build a `break-glass/{beaconId}` route. Beacon IDs may contain `:`
     * which is not a path-segment-safe character; we URL-encode here and
     * the screen decodes via the navArg.
     */
    fun breakGlass(beaconId: String): String =
        "break-glass/${java.net.URLEncoder.encode(beaconId, Charsets.UTF_8.name())}"

    fun patient(caseUlid: String): String = "patient/$caseUlid"
    fun intervention(caseUlid: String): String = "intervention/$caseUlid"
    fun timeline(caseUlid: String): String = "timeline/$caseUlid"
    fun handoff(caseUlid: String): String = "handoff/$caseUlid"

    fun decodeBeaconId(raw: String): String =
        java.net.URLDecoder.decode(raw, Charsets.UTF_8.name())

    const val ARG_BEACON_ID = "beaconId"
    const val ARG_CASE_ULID = "caseUlid"
}
