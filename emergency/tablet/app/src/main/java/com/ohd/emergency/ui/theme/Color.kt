package com.ohd.emergency.ui.theme

import androidx.compose.ui.graphics.Color

/**
 * OHD Emergency palette.
 *
 * Mirrors the OHD Connect palette (Red / Ink / White / Muted) per
 * `ux-design.md` "Design Aesthetic", but tuned for the paramedic-tablet
 * context: **dark mode is the only target**, and the red accent is
 * dialled brighter so it remains legible under direct sunlight (ambulance
 * windscreen / outside daytime call) and through gloves.
 *
 * Where Connect's palette aims for "calm, restrained tool", Emergency's
 * palette aims for "high-contrast, never-miss-a-beat" — the same brand
 * vocabulary applied to a 10" landscape tablet that must remain readable
 * across the gamut from 04:00 ambulance interior to noon-sun roadside.
 *
 * Colour roles:
 *   - Red (`Critical`): emergency / urgent / break-glass affordance.
 *     Red-bordered cards (allergies, advance directives) and the
 *     primary action button on the break-glass dialog.
 *   - Amber (`AutoGrant`): timeout-default-allow indicator. Distinct
 *     from urgent red so the responder can tell at a glance whether the
 *     patient actively approved or whether access was auto-granted.
 *     Specific design note from `screens-emergency.md` "Designer's
 *     handoff notes".
 *   - Ink black background, near-black surface variants, white-on-dark
 *     primary text.
 */
internal object EmergencyPalette {
    // Brand / urgent
    val Red = Color(0xFFE11D2A)         // primary brand red (matches Connect)
    val RedBright = Color(0xFFFF4651)   // dark-mode-friendly variant; default primary
    val RedDeep = Color(0xFFB71721)     // pressed / focused

    // Auto-grant indicator (amber) — for timeout-default-allow cases.
    // See `spec/screens-emergency.md` "Designer's handoff notes":
    //     The auto-granted badge needs a distinct visual treatment —
    //     different color (perhaps amber or muted red), small icon …
    // We pick a saturated amber that contrasts with the Red while
    // staying legible on dark backgrounds.
    val AutoGrant = Color(0xFFE7A50D)
    val AutoGrantSoft = Color(0xFF8A5C00)

    // Status semantics
    val Success = Color(0xFF2E7D32)         // approved, queued-flushed-ok, sync ok
    val SuccessSoft = Color(0xFF1B5E20)
    val Warning = Color(0xFFB26A00)         // queued-pending, partial sync
    val Info = Color(0xFF0288D1)            // neutral status chips

    // Greyscale
    val Ink = Color(0xFF050505)             // background — slightly inkier than Connect
    val White = Color(0xFFFFFFFF)
    val MutedDark = Color(0xFFB0B0B0)       // secondary text on dark
    val MutedDarker = Color(0xFF6F6F6F)     // tertiary text / disabled
    val OutlineDark = Color(0xFF333333)
    val SurfaceDark = Color(0xFF101010)
    val SurfaceVariantDark = Color(0xFF1B1B1B)
    val SurfaceElevatedDark = Color(0xFF242424)

    // Light scheme (provided so the theme engine doesn't crash if the
    // OS forces light, but the app always launches dark).
    val SurfaceLight = Color(0xFFFAFAFA)
    val SurfaceVariantLight = Color(0xFFF1F1F1)
    val OutlineLight = Color(0xFFD9D9D9)
    val MutedLight = Color(0xFF555555)
}
