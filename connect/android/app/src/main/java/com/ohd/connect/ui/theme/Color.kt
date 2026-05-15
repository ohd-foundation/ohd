package com.ohd.connect.ui.theme

import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.graphics.Color

/**
 * OHD Connect colour tokens.
 *
 * Source of truth: `spec/design/_export/SPEC.md` §1. The token names match
 * the spec one-for-one. We expose them as flat `val`s on `OhdColors` and
 * also wire the brand-relevant subset into a Material3 [lightColorScheme]
 * via [OhdLightColorScheme] so library composables that read
 * `MaterialTheme.colorScheme` get the right palette.
 *
 * **Light theme only** for v1 — there is no dark variant.
 */
object OhdColors {
    // Surfaces
    val Bg: Color = Color(0xFFFFFFFF)         // ohd-bg
    val BgElevated: Color = Color(0xFFFAFAFA) // ohd-bg-elevated

    // Ink (text)
    val Ink: Color = Color(0xFF0A0A0A)        // ohd-ink
    val InkSoft: Color = Color(0xFF3A3A3A)    // ohd-ink-soft

    // Lines / dividers
    val Line: Color = Color(0xFFE5E5E5)       // ohd-line
    val LineSoft: Color = Color(0xFFF2F2F2)   // ohd-line-soft

    // Muted (secondary text, icons)
    val Muted: Color = Color(0xFF6B6B6B)      // ohd-muted

    // Brand reds
    val Red: Color = Color(0xFFE11D2A)        // ohd-red
    val RedDark: Color = Color(0xFFB5121E)    // ohd-red-dark
    val RedTint: Color = Color(0xFFFCE6E8)    // ohd-red-tint

    // Semantic
    val Success: Color = Color(0xFF1F8E4A)    // ohd-success
    val Warn: Color = Color(0xFFB57500)       // ohd-warn

    // Convenience
    val White: Color = Color(0xFFFFFFFF)
}

/**
 * Material3 colour scheme wired to the OHD palette.
 *
 * The mapping prioritises components inside `ui/components/` so they can
 * read `MaterialTheme.colorScheme.primary` etc. without bypassing M3.
 *
 * - `primary` → `ohd-red` (CTA fill)
 * - `onPrimary` → white
 * - `background` / `surface` → `ohd-bg`
 * - `surfaceVariant` → `ohd-bg-elevated` (cards/panels)
 * - `outline` → `ohd-line`, `outlineVariant` → `ohd-line-soft`
 * - `onSurface` → `ohd-ink`, `onSurfaceVariant` → `ohd-muted`
 * - `error` → `ohd-red-dark` (destructive action surface)
 */
internal val OhdLightColorScheme = lightColorScheme(
    primary = OhdColors.Red,
    onPrimary = OhdColors.White,
    primaryContainer = OhdColors.RedTint,
    onPrimaryContainer = OhdColors.RedDark,

    secondary = OhdColors.Ink,
    onSecondary = OhdColors.White,

    background = OhdColors.Bg,
    onBackground = OhdColors.Ink,
    surface = OhdColors.Bg,
    onSurface = OhdColors.Ink,
    surfaceVariant = OhdColors.BgElevated,
    onSurfaceVariant = OhdColors.Muted,

    outline = OhdColors.Line,
    outlineVariant = OhdColors.LineSoft,

    error = OhdColors.RedDark,
    onError = OhdColors.White,
)

// -----------------------------------------------------------------------------
// Backwards-compatible aliases
// -----------------------------------------------------------------------------
//
// The pre-rebuild palette had a different shape (mixed-mode dark/light, named
// after their role rather than the spec token). Existing operator screens
// (Setup/Grants/Pending/Cases/Audit/Emergency/Export) reference some of the
// old names indirectly; the alias below keeps them compiling. New code should
// use `OhdColors` directly.

internal object OhdPalette {
    val Red = OhdColors.Red
    val RedDeep = OhdColors.RedDark
    val RedSoft = OhdColors.Red
    val Ink = OhdColors.Ink
    val White = OhdColors.White
    val MutedLight = OhdColors.Muted
    val MutedDark = OhdColors.Muted
    val SurfaceLight = OhdColors.Bg
    val SurfaceVariantLight = OhdColors.BgElevated
    val OutlineLight = OhdColors.Line
    val Success = OhdColors.Success
    val Warning = OhdColors.Warn
}
