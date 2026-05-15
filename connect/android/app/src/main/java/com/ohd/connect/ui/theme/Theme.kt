package com.ohd.connect.ui.theme

import android.app.Activity
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalView
import androidx.core.view.WindowCompat

/**
 * Top-level Compose theme.
 *
 * Light theme only for v1 — see spec §1: "Light theme primary, no theme
 * switching for v1." Status bar tint matches `ohd-bg` (white) with dark
 * status-bar icons.
 *
 * Two entry points:
 *   - [OhdTheme] is the canonical name per spec §5.
 *   - [OhdConnectTheme] is the legacy name; the existing operator screens
 *     (Setup/Grants/Pending/Cases/Audit/Emergency/Export) reference it for
 *     `@Preview` blocks. Both delegate to the same internal implementation.
 */
@Composable
fun OhdTheme(content: @Composable () -> Unit) {
    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            val window = (view.context as? Activity)?.window
            if (window != null) {
                window.statusBarColor = OhdColors.Bg.toArgb()
                window.navigationBarColor = OhdColors.Bg.toArgb()
                val insets = WindowCompat.getInsetsController(window, view)
                insets.isAppearanceLightStatusBars = true
                insets.isAppearanceLightNavigationBars = true
            }
        }
    }

    MaterialTheme(
        colorScheme = OhdLightColorScheme,
        typography = OhdTypography,
        shapes = OhdShapes,
        content = content,
    )
}

/**
 * Backwards-compatible alias for the existing operator screens whose
 * `@Preview` blocks reference [OhdConnectTheme]. New code should call
 * [OhdTheme] directly.
 */
@Composable
fun OhdConnectTheme(content: @Composable () -> Unit) = OhdTheme(content)
