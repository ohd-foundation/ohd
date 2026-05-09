package com.ohd.connect.ui.theme

import androidx.compose.ui.graphics.Color

/**
 * OHD Connect palette.
 *
 * Per `ux-design.md` "Design Aesthetic": clean, professional, restrained.
 * Black / white / red as the dominant palette; muted grey for secondary
 * text. No decorative colours, no gradients, no playful accents — the app
 * tracks sensitive medical events and should feel like a tool, not a
 * lifestyle wellness gimmick.
 */
internal object OhdPalette {
    // Brand
    val Red = Color(0xFFE11D2A)         // primary accent, brand
    val RedDeep = Color(0xFFB71721)     // pressed / focused
    val RedSoft = Color(0xFFFF4651)     // dark-mode-friendly variant

    // Greyscale (the spec calls these Ink / White / Muted)
    val Ink = Color(0xFF0A0A0A)         // text, icons (light mode)
    val White = Color(0xFFFFFFFF)
    val MutedLight = Color(0xFF6B6B6B)  // secondary text on white
    val MutedDark = Color(0xFFA1A1A1)   // secondary text on black

    // Surfaces — dark mode is the default per `ux-design.md`.
    val SurfaceDark = Color(0xFF0A0A0A)
    val SurfaceVariantDark = Color(0xFF181818)
    val OutlineDark = Color(0xFF2A2A2A)

    val SurfaceLight = Color(0xFFFFFFFF)
    val SurfaceVariantLight = Color(0xFFF5F5F5)
    val OutlineLight = Color(0xFFE0E0E0)

    // Semantic — desaturated per the spec.
    val Success = Color(0xFF2E7D32)
    val Warning = Color(0xFFB26A00)
}
