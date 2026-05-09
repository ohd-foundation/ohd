package com.ohd.emergency.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/**
 * Typography stack — Emergency-tuned.
 *
 * Mirrors Connect's font stack (Outfit / Inter / JetBrains Mono via the
 * system fallback in v0) but **scales every step up** so a paramedic in
 * gloves can read patient data from a chest-mounted tablet at arm's
 * length. The default Material3 type scale assumes a phone-in-hand
 * reading distance (~30 cm). Tablet-on-stretcher reading distance is
 * ~50–70 cm; we add roughly +25% to display, +15% to body.
 *
 * `MonoStyle` is reserved for ULIDs, channel paths, dose values, and
 * vital-sign numbers — anywhere a paramedic might need to confirm a
 * digit-by-digit value at a glance.
 *
 * TODO: drop `Outfit-VariableFont_wght.ttf`, `Inter-VariableFont.ttf`,
 *       `JetBrainsMono-VariableFont_wght.ttf` into res/font/ and wire
 *       a `Font(R.font.outfit, ...)`-based `FontFamily` here. Same
 *       upgrade path as connect/android/Type.kt.
 */
internal val EmergencyTypography: Typography = Typography().run {
    val display = FontFamily.SansSerif      // → Outfit (light weight; brand-display)
    val body = FontFamily.SansSerif         // → Inter
    val mono = FontFamily.Monospace         // → JetBrains Mono

    Typography(
        // Display — used on the break-glass countdown timer and the case
        // header. Bold rather than light because the screen needs to be
        // readable across light conditions.
        displayLarge = displayLarge.copy(
            fontFamily = display,
            fontWeight = FontWeight.SemiBold,
            fontSize = 72.sp,
            lineHeight = 80.sp,
        ),
        displayMedium = displayMedium.copy(
            fontFamily = display,
            fontWeight = FontWeight.SemiBold,
            fontSize = 56.sp,
            lineHeight = 64.sp,
        ),
        displaySmall = displaySmall.copy(
            fontFamily = display,
            fontWeight = FontWeight.Medium,
            fontSize = 44.sp,
            lineHeight = 52.sp,
        ),

        headlineLarge = headlineLarge.copy(
            fontFamily = display,
            fontWeight = FontWeight.SemiBold,
            fontSize = 36.sp,
        ),
        headlineMedium = headlineMedium.copy(
            fontFamily = display,
            fontWeight = FontWeight.Medium,
            fontSize = 30.sp,
        ),
        headlineSmall = headlineSmall.copy(
            fontFamily = display,
            fontWeight = FontWeight.Medium,
            fontSize = 24.sp,
        ),

        titleLarge = titleLarge.copy(
            fontFamily = body,
            fontWeight = FontWeight.SemiBold,
            fontSize = 22.sp,
        ),
        titleMedium = titleMedium.copy(
            fontFamily = body,
            fontWeight = FontWeight.Medium,
            fontSize = 18.sp,
        ),
        titleSmall = titleSmall.copy(
            fontFamily = body,
            fontWeight = FontWeight.Medium,
            fontSize = 16.sp,
        ),

        // Body — bumped from defaults (16/14/12) so the patient-view scroll
        // stays glanceable.
        bodyLarge = bodyLarge.copy(fontFamily = body, fontSize = 18.sp),
        bodyMedium = bodyMedium.copy(fontFamily = body, fontSize = 16.sp),
        bodySmall = bodySmall.copy(fontFamily = body, fontSize = 14.sp),

        labelLarge = labelLarge.copy(fontFamily = body, fontWeight = FontWeight.Medium, fontSize = 16.sp),
        labelMedium = labelMedium.copy(fontFamily = body, fontWeight = FontWeight.Medium, fontSize = 14.sp),
        labelSmall = labelSmall.copy(fontFamily = body, fontWeight = FontWeight.Medium, fontSize = 12.sp),
    )
}

/** Monospace — vitals values, ULIDs, channel paths, dose values. */
internal val MonoStyle: TextStyle = TextStyle(
    fontFamily = FontFamily.Monospace,
    fontSize = 16.sp,
    letterSpacing = 0.sp,
)

/** Big-number monospace — for the vitals pad readout. */
internal val BigNumberStyle: TextStyle = TextStyle(
    fontFamily = FontFamily.Monospace,
    fontSize = 56.sp,
    fontWeight = FontWeight.Medium,
    letterSpacing = 0.sp,
)

/** Countdown timer style — break-glass dialog. */
internal val CountdownStyle: TextStyle = TextStyle(
    fontFamily = FontFamily.SansSerif,
    fontSize = 96.sp,
    fontWeight = FontWeight.SemiBold,
    letterSpacing = 0.sp,
)
