package com.ohd.connect.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/**
 * Typography stack.
 *
 * `ux-design.md` calls for Outfit (display, weight 100–300), Inter (body),
 * JetBrains Mono (data values, tokens). For the v0 scaffold we route every
 * weight through the system stack — the Outfit / Inter / JetBrains Mono
 * font files land alongside `app/src/main/res/font/` in implementation
 * phase. The `FontFamily.SansSerif` / `FontFamily.Monospace` fallbacks
 * keep the typographic hierarchy intact even before the custom fonts ship.
 *
 * TODO: drop `Outfit-VariableFont_wght.ttf`, `Inter-VariableFont.ttf`,
 *       `JetBrainsMono-VariableFont_wght.ttf` into res/font/ and wire up
 *       a `Font(R.font.outfit, ...)`-based `FontFamily` here.
 */
internal val OhdTypography: Typography = Typography().run {
    val display = FontFamily.SansSerif      // → Outfit, weight 100–300
    val body = FontFamily.SansSerif         // → Inter
    val mono = FontFamily.Monospace         // → JetBrains Mono

    Typography(
        displayLarge = displayLarge.copy(fontFamily = display, fontWeight = FontWeight.Light),
        displayMedium = displayMedium.copy(fontFamily = display, fontWeight = FontWeight.Light),
        displaySmall = displaySmall.copy(fontFamily = display, fontWeight = FontWeight.Light),

        headlineLarge = headlineLarge.copy(fontFamily = display, fontWeight = FontWeight.Light),
        headlineMedium = headlineMedium.copy(fontFamily = display, fontWeight = FontWeight.Light),
        headlineSmall = headlineSmall.copy(fontFamily = display, fontWeight = FontWeight.Normal),

        titleLarge = titleLarge.copy(fontFamily = body, fontWeight = FontWeight.Medium),
        titleMedium = titleMedium.copy(fontFamily = body, fontWeight = FontWeight.Medium),
        titleSmall = titleSmall.copy(fontFamily = body, fontWeight = FontWeight.Medium),

        bodyLarge = bodyLarge.copy(fontFamily = body),
        bodyMedium = bodyMedium.copy(fontFamily = body),
        bodySmall = bodySmall.copy(fontFamily = body),

        labelLarge = labelLarge.copy(fontFamily = body, fontWeight = FontWeight.Medium),
        labelMedium = labelMedium.copy(fontFamily = body, fontWeight = FontWeight.Medium),
        labelSmall = labelSmall.copy(fontFamily = body, fontWeight = FontWeight.Medium),
    )
}

/** Monospace text style for ULIDs, channel values, tokens. */
internal val MonoStyle: TextStyle = TextStyle(
    fontFamily = FontFamily.Monospace,
    fontSize = 13.sp,
    letterSpacing = 0.sp,
)
