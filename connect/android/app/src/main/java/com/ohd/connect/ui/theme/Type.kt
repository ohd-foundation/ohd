package com.ohd.connect.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/**
 * Typography stack — Outfit (display), Inter (body), JetBrains Mono (numerics).
 *
 * The Pencil designs reference three Google Fonts families:
 *   - Outfit         (200/300/400/500) — wordmark, hero numbers, screen titles.
 *   - Inter          (400/500/600)     — body text, labels, buttons.
 *   - JetBrains Mono (400/500)         — numeric values, units, ULIDs.
 *
 * For the v1 design-system landing we route every family through the system
 * stack (`FontFamily.SansSerif` / `FontFamily.Monospace`) so the build never
 * blocks on a network-served font fetch. The resulting hierarchy is correct
 * (Outfit/Inter both render as the system sans-serif, but at distinct
 * weights), and a follow-up commit can swap any of the three to either
 * downloadable Google Fonts or .ttf resources without touching the call
 * sites — `OhdDisplay` / `OhdBody` / `OhdMono` are the only public handles.
 *
 * **To swap to .ttf resources:**
 *   1. Drop `Outfit-VariableFont_wght.ttf`, `Inter-VariableFont.ttf`, and
 *      `JetBrainsMono-VariableFont_wght.ttf` into `app/src/main/res/font/`.
 *   2. Replace the `FontFamily.SansSerif` / `FontFamily.Monospace` defaults
 *      below with `FontFamily(Font(R.font.outfit, weight = FontWeight.W200), …)`.
 */

val OhdDisplay: FontFamily = FontFamily.SansSerif // → Outfit (200/300/400/500)
val OhdBody: FontFamily = FontFamily.SansSerif    // → Inter (400/500/600)
val OhdMono: FontFamily = FontFamily.Monospace    // → JetBrains Mono (400/500)

/**
 * Material3 typography mapping.
 *
 * Component-specific sizes (the 32 sp Outfit 200 stat-tile number, the
 * 11 sp uppercase section header, …) live on the components themselves —
 * the styles below are sensible defaults for `MaterialTheme.typography.X`.
 */
internal val OhdTypography: Typography = Typography(
    // Display — Outfit, used for hero/wordmark.
    displayLarge = TextStyle(fontFamily = OhdDisplay, fontWeight = FontWeight.W200, fontSize = 36.sp),
    displayMedium = TextStyle(fontFamily = OhdDisplay, fontWeight = FontWeight.W200, fontSize = 32.sp),
    displaySmall = TextStyle(fontFamily = OhdDisplay, fontWeight = FontWeight.W300, fontSize = 28.sp),

    // Headline — Outfit, screen titles / large numbers.
    headlineLarge = TextStyle(fontFamily = OhdDisplay, fontWeight = FontWeight.W300, fontSize = 24.sp),
    headlineMedium = TextStyle(fontFamily = OhdDisplay, fontWeight = FontWeight.W300, fontSize = 22.sp),
    headlineSmall = TextStyle(fontFamily = OhdDisplay, fontWeight = FontWeight.W400, fontSize = 20.sp),

    // Title — Inter, top-bar, list-item primary, card title.
    titleLarge = TextStyle(fontFamily = OhdBody, fontWeight = FontWeight.W500, fontSize = 17.sp),
    titleMedium = TextStyle(fontFamily = OhdBody, fontWeight = FontWeight.W600, fontSize = 15.sp),
    titleSmall = TextStyle(fontFamily = OhdBody, fontWeight = FontWeight.W500, fontSize = 14.sp),

    // Body — Inter, narrative text and most labels.
    bodyLarge = TextStyle(fontFamily = OhdBody, fontWeight = FontWeight.W400, fontSize = 15.sp),
    bodyMedium = TextStyle(fontFamily = OhdBody, fontWeight = FontWeight.W400, fontSize = 14.sp),
    bodySmall = TextStyle(fontFamily = OhdBody, fontWeight = FontWeight.W400, fontSize = 12.sp),

    // Label — Inter, small uppercase / button text.
    labelLarge = TextStyle(fontFamily = OhdBody, fontWeight = FontWeight.W500, fontSize = 14.sp),
    labelMedium = TextStyle(fontFamily = OhdBody, fontWeight = FontWeight.W500, fontSize = 13.sp),
    labelSmall = TextStyle(fontFamily = OhdBody, fontWeight = FontWeight.W500, fontSize = 11.sp, letterSpacing = 2.sp),
)

/**
 * Monospace text style for ULIDs, channel values, tokens. Re-exposed here so
 * the existing Settings / Dashboard / Export screens keep compiling.
 */
val MonoStyle: TextStyle = TextStyle(
    fontFamily = OhdMono,
    fontWeight = FontWeight.W400,
    fontSize = 13.sp,
    letterSpacing = 0.sp,
)
