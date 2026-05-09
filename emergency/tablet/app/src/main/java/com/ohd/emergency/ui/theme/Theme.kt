package com.ohd.emergency.ui.theme

import android.app.Activity
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat

/**
 * App-wide Material3 theme for OHD Emergency.
 *
 * Forces **dark mode** regardless of system setting — paramedics work in
 * night shifts, ambulance interiors with red/blue-flashing strobes, and
 * outdoor low-light. A user-toggle for light mode is intentionally not
 * exposed; it would add complexity and the dark scheme is the right
 * default for every documented call scenario.
 *
 * Material You / dynamic colour is **disabled** — the brand red and the
 * amber auto-grant indicator are load-bearing semantically. Letting the
 * user's wallpaper bleed into a medical tool's chrome would be an
 * accessibility risk (a green-tinted "approve" button could confuse a
 * paramedic looking for the urgent-red affordance).
 */

private val DarkColors = darkColorScheme(
    // Primary = bright red. Used for the break-glass primary action,
    // critical-info card border, and the active-case banner accent.
    primary = EmergencyPalette.RedBright,
    onPrimary = EmergencyPalette.White,
    primaryContainer = EmergencyPalette.RedDeep,
    onPrimaryContainer = EmergencyPalette.White,

    // Secondary = the auto-grant amber. Used by status chips that
    // signal "auto-granted via timeout" (per `screens-emergency.md`).
    secondary = EmergencyPalette.AutoGrant,
    onSecondary = EmergencyPalette.Ink,
    secondaryContainer = EmergencyPalette.AutoGrantSoft,
    onSecondaryContainer = EmergencyPalette.White,

    // Tertiary = neutral info blue. Reserved for "general info" status
    // chips so they stay visually distinct from urgent red and amber.
    tertiary = EmergencyPalette.Info,
    onTertiary = EmergencyPalette.White,

    background = EmergencyPalette.Ink,
    onBackground = EmergencyPalette.White,
    surface = EmergencyPalette.SurfaceDark,
    onSurface = EmergencyPalette.White,
    surfaceVariant = EmergencyPalette.SurfaceVariantDark,
    onSurfaceVariant = EmergencyPalette.MutedDark,
    surfaceContainerHighest = EmergencyPalette.SurfaceElevatedDark,

    outline = EmergencyPalette.OutlineDark,
    outlineVariant = EmergencyPalette.OutlineDark,

    error = EmergencyPalette.RedBright,
    onError = EmergencyPalette.White,
)

private val LightColors = lightColorScheme(
    primary = EmergencyPalette.Red,
    onPrimary = EmergencyPalette.White,
    primaryContainer = EmergencyPalette.Red,
    onPrimaryContainer = EmergencyPalette.White,
    secondary = EmergencyPalette.AutoGrant,
    onSecondary = EmergencyPalette.Ink,
    background = EmergencyPalette.SurfaceLight,
    onBackground = EmergencyPalette.Ink,
    surface = EmergencyPalette.SurfaceLight,
    onSurface = EmergencyPalette.Ink,
    surfaceVariant = EmergencyPalette.SurfaceVariantLight,
    onSurfaceVariant = EmergencyPalette.MutedLight,
    outline = EmergencyPalette.OutlineLight,
    error = EmergencyPalette.Red,
    onError = EmergencyPalette.White,
)

@Composable
fun EmergencyTheme(
    // Default to dark; override possible for previews and tests.
    darkTheme: Boolean = true,
    content: @Composable () -> Unit,
) {
    val colorScheme = if (darkTheme) DarkColors else LightColors

    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as? Activity)?.window
            if (window != null) {
                window.statusBarColor = Color.Transparent.toArgb()
                window.navigationBarColor = colorScheme.background.toArgb()
                WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars =
                    !darkTheme
                WindowCompat.getInsetsController(window, view).isAppearanceLightNavigationBars =
                    !darkTheme
            }
        }
    }

    MaterialTheme(
        colorScheme = colorScheme,
        typography = EmergencyTypography,
        content = content,
    )
}
