package com.ohd.connect.ui.icons

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.ArrowBack
import androidx.compose.material.icons.outlined.ArrowUpward
import androidx.compose.material.icons.outlined.AutoAwesome
import androidx.compose.material.icons.outlined.Cloud
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.Domain
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.Favorite
import androidx.compose.material.icons.outlined.FitnessCenter
import androidx.compose.material.icons.outlined.History
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.LocalFireDepartment
import androidx.compose.material.icons.outlined.LocalPharmacy
import androidx.compose.material.icons.outlined.Medication
import androidx.compose.material.icons.outlined.MonitorHeart
import androidx.compose.material.icons.outlined.Notifications
import androidx.compose.material.icons.outlined.Numbers
import androidx.compose.material.icons.outlined.Opacity
import androidx.compose.material.icons.outlined.PhoneAndroid
import androidx.compose.material.icons.outlined.QrCodeScanner
import androidx.compose.material.icons.outlined.Restaurant
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Shield
import androidx.compose.material.icons.outlined.Storage
import androidx.compose.material.icons.outlined.Thermostat
import androidx.compose.material.icons.outlined.VerifiedUser
import androidx.compose.material.icons.outlined.DragHandle
import androidx.compose.ui.graphics.vector.ImageVector

/**
 * Lucide icon name → Material Icons mapping.
 *
 * The Pencil designs reference Lucide icons (e.g. `lucide:pill`,
 * `lucide:utensils`) which Material Icons doesn't ship 1:1. Each entry below
 * is the closest visual approximation from `material-icons-extended`. Where
 * Material has no near match (e.g. `scan-barcode`, `grip-vertical`) we pick
 * the next-best stand-in and document the choice inline.
 *
 * **Centralised** so that screens reference `OhdIcons.Pill` (not a Material
 * import directly). When a Lucide-Compose library lands we swap the right-
 * hand sides without touching any call sites.
 */
object OhdIcons {
    // Navigation / chrome
    val Home: ImageVector = Icons.Outlined.Home
    val Plus: ImageVector = Icons.Outlined.Add
    val History: ImageVector = Icons.Outlined.History
    val Settings: ImageVector = Icons.Outlined.Settings
    val ArrowLeft: ImageVector = Icons.Outlined.ArrowBack
    val ArrowUp: ImageVector = Icons.Outlined.ArrowUpward
    val ChevronRight: ImageVector = Icons.Outlined.ChevronRight
    val ChevronDown: ImageVector = Icons.Outlined.KeyboardArrowDown

    // Quick-log surfaces
    val Pill: ImageVector = Icons.Outlined.Medication            // lucide:pill
    val Utensils: ImageVector = Icons.Outlined.Restaurant        // lucide:utensils
    val Activity: ImageVector = Icons.Outlined.MonitorHeart      // lucide:activity (ECG-style line; MonitorHeart is the Material analogue)
    val Thermometer: ImageVector = Icons.Outlined.Thermostat     // lucide:thermometer
    val Droplets: ImageVector = Icons.Outlined.Opacity           // lucide:droplets (drop)
    val HeartPulse: ImageVector = Icons.Outlined.Favorite        // lucide:heart-pulse (no exact pulse variant)
    val Dumbbell: ImageVector = Icons.Outlined.FitnessCenter     // lucide:dumbbell

    // Settings hub
    val Database: ImageVector = Icons.Outlined.Storage           // lucide:database
    val Shield: ImageVector = Icons.Outlined.Shield              // lucide:shield
    val ShieldCheck: ImageVector = Icons.Outlined.VerifiedUser   // lucide:shield-check
    val FileText: ImageVector = Icons.Outlined.Description       // lucide:file-text
    val Bell: ImageVector = Icons.Outlined.Notifications         // lucide:bell
    val Sparkles: ImageVector = Icons.Outlined.AutoAwesome       // lucide:sparkles

    // Storage option icons
    val Smartphone: ImageVector = Icons.Outlined.PhoneAndroid    // lucide:smartphone
    val Cloud: ImageVector = Icons.Outlined.Cloud                // lucide:cloud
    val Server: ImageVector = Icons.Outlined.Dns                 // lucide:server
    val Building2: ImageVector = Icons.Outlined.Domain           // lucide:building-2

    // Form-builder / measurement
    val Hash: ImageVector = Icons.Outlined.Numbers               // lucide:hash
    val GripVertical: ImageVector = Icons.Outlined.DragHandle    // lucide:grip-vertical (DragHandle is horizontal-ish but is the canonical Material drag affordance)
    val ScanBarcode: ImageVector = Icons.Outlined.QrCodeScanner  // lucide:scan-barcode (QR ≈ barcode in Material)
    val Pharmacy: ImageVector = Icons.Outlined.LocalPharmacy     // alt for medication on-hand
    val Flame: ImageVector = Icons.Outlined.LocalFireDepartment  // for energy/calories accents

    // Affordances
    val Edit: ImageVector = Icons.Outlined.Edit                  // lucide:pencil (Material's "edit" is a pencil)
}

/**
 * Per-event-type visual chrome (icon + tint).
 *
 * The Recent Events timeline renders one of these per row so each
 * namespace is visually distinct at a glance. Keep tints consistent with
 * the brand palette: red for "core medical" surfaces (measurement /
 * medication / symptom / emergency), warm orange for food (to keep food
 * rows from blending into measurement reds), green for activity.
 */
data class EventVisual(
    val icon: ImageVector,
    val tint: androidx.compose.ui.graphics.Color,
)

/**
 * Map a flat `"namespace.name"` event type to its [EventVisual].
 *
 * Resolution is by **namespace prefix** so an unknown leaf inside a known
 * namespace (e.g. a future `measurement.cortisol`) still picks up the
 * right family icon. Unknown namespaces fall through to a generic
 * file-text glyph at muted tint.
 */
fun visualFor(eventType: String): EventVisual {
    val ns = eventType.substringBefore('.', missingDelimiterValue = eventType)
    return when (ns) {
        "measurement" -> EventVisual(OhdIcons.Activity, com.ohd.connect.ui.theme.OhdColors.Red)
        "medication" -> EventVisual(OhdIcons.Pill, com.ohd.connect.ui.theme.OhdColors.Red)
        "food" -> EventVisual(OhdIcons.Utensils, androidx.compose.ui.graphics.Color(0xFFE07A1B))
        "symptom" -> EventVisual(OhdIcons.Thermometer, com.ohd.connect.ui.theme.OhdColors.Red)
        "activity" -> EventVisual(OhdIcons.Dumbbell, androidx.compose.ui.graphics.Color(0xFF1F8E4A))
        "emergency" -> EventVisual(OhdIcons.ShieldCheck, com.ohd.connect.ui.theme.OhdColors.Red)
        else -> EventVisual(OhdIcons.FileText, com.ohd.connect.ui.theme.OhdColors.Muted)
    }
}

/** Just the icon — convenience wrapper around [visualFor]. */
fun iconFor(eventType: String): ImageVector = visualFor(eventType).icon
