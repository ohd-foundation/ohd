package com.ohd.connect.ui.theme

import android.app.Activity
import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat

/**
 * App-wide Material3 theme.
 *
 * Defaults to **dark mode** per `ux-design.md` ("Design Aesthetic"). The
 * light scheme is provided so the system setting still works for users who
 * prefer it; auto-following the system is the right default behaviour.
 *
 * Dynamic colour (Material You) is **disabled** intentionally — the spec
 * pins a brand-specific palette and we don't want the user's home-screen
 * wallpaper to seep into a medical tool's chrome.
 */

private val DarkColors = darkColorScheme(
    primary = OhdPalette.RedSoft,
    onPrimary = OhdPalette.White,
    primaryContainer = OhdPalette.RedDeep,
    onPrimaryContainer = OhdPalette.White,

    secondary = OhdPalette.MutedDark,
    onSecondary = OhdPalette.Ink,

    background = OhdPalette.SurfaceDark,
    onBackground = OhdPalette.White,
    surface = OhdPalette.SurfaceDark,
    onSurface = OhdPalette.White,
    surfaceVariant = OhdPalette.SurfaceVariantDark,
    onSurfaceVariant = OhdPalette.MutedDark,

    outline = OhdPalette.OutlineDark,
    outlineVariant = OhdPalette.OutlineDark,

    error = OhdPalette.RedSoft,
    onError = OhdPalette.White,
)

private val LightColors = lightColorScheme(
    primary = OhdPalette.Red,
    onPrimary = OhdPalette.White,
    primaryContainer = OhdPalette.Red,
    onPrimaryContainer = OhdPalette.White,

    secondary = OhdPalette.MutedLight,
    onSecondary = OhdPalette.White,

    background = OhdPalette.SurfaceLight,
    onBackground = OhdPalette.Ink,
    surface = OhdPalette.SurfaceLight,
    onSurface = OhdPalette.Ink,
    surfaceVariant = OhdPalette.SurfaceVariantLight,
    onSurfaceVariant = OhdPalette.MutedLight,

    outline = OhdPalette.OutlineLight,
    outlineVariant = OhdPalette.OutlineLight,

    error = OhdPalette.Red,
    onError = OhdPalette.White,
)

@Composable
fun OhdConnectTheme(
    // Default = follow the system; we hint dark in `MainActivity` by setting
    // the activity's UI mode, but the theme honours the system override.
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    val colorScheme = if (darkTheme) DarkColors else LightColors

    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as? Activity)?.window
            if (window != null) {
                window.statusBarColor = colorScheme.background.toArgb()
                window.navigationBarColor = colorScheme.background.toArgb()
                WindowCompat.getInsetsController(window, view).isAppearanceLightStatusBars =
                    !darkTheme
                WindowCompat.getInsetsController(window, view).isAppearanceLightNavigationBars =
                    !darkTheme
            }
        }
    }

    // Dynamic colour deliberately not used (see header comment).
    @Suppress("UNUSED_VARIABLE")
    val _supportsDynamic = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S

    MaterialTheme(
        colorScheme = colorScheme,
        typography = OhdTypography,
        content = content,
    )
}
