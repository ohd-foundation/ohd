package com.ohd.connect.ui.screens._shared

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.launch

/**
 * Preset favourites the user can pin to the Home strip with one tap.
 *
 * @param label Display string shown on the chip.
 * @param kind  Stable token persisted to JSON; matches the `?preselect=`
 *              token consumed by [com.ohd.connect.ui.screens.MeasurementScreen]
 *              via `favouriteToPreselect` / `parsePreselect`.
 * @param iconKey Stable token persisted to JSON; resolved at render time via
 *              [HomeFavourite.resolveIcon]. Keeping the symbolic key in the
 *              prefs blob (rather than serializing an [ImageVector]) means
 *              the on-disk shape survives icon refactors.
 */
data class FavouritePreset(
    val label: String,
    val kind: String,
    val iconKey: String,
    val icon: ImageVector,
)

private val PRESETS: List<FavouritePreset> = listOf(
    FavouritePreset("Blood pressure", "blood_pressure", "heart_pulse", OhdIcons.HeartPulse),
    FavouritePreset("Glucose", "glucose", "droplets", OhdIcons.Droplets),
    FavouritePreset("Weight", "weight", "activity", OhdIcons.Activity),
    FavouritePreset("Temperature", "temperature", "thermometer", OhdIcons.Thermometer),
    FavouritePreset("Heart rate", "heart_rate", "heart_pulse", OhdIcons.HeartPulse),
    FavouritePreset("SpO2", "spo2", "droplets", OhdIcons.Droplets),
)

/**
 * Modal bottom sheet for picking a new Home favourite.
 *
 * Presents the six [PRESETS] then a free-text "Custom" row whose label
 * lands as kind `custom`. The caller (HomeScreen) persists the chosen
 * entry by appending it to the `home_favourites_v1` JSON array via
 * [com.ohd.connect.data.Auth.saveHomeFavouritesJson].
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AddFavouriteSheet(
    onDismiss: () -> Unit,
    onPick: (label: String, kind: String, iconKey: String) -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    var customLabel by remember { mutableStateOf("") }

    val pickThen: (FavouritePreset) -> Unit = { preset ->
        scope.launch { sheetState.hide() }.invokeOnCompletion {
            onDismiss()
            onPick(preset.label, preset.kind, preset.iconKey)
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = OhdColors.Bg,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = "ADD FAVOURITE",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 11.sp,
                letterSpacing = 2.sp,
                color = OhdColors.Muted,
                modifier = Modifier.padding(horizontal = 4.dp, vertical = 8.dp),
            )

            PRESETS.forEach { preset ->
                PresetRow(preset = preset, onClick = { pickThen(preset) })
            }

            Spacer(Modifier.height(8.dp))

            // Custom — free-text label, defaults to the user's typed string
            // and an `activity` icon. Persists kind `custom` so the
            // measurement screen falls back to the un-preselected list.
            OhdField(
                label = "Custom",
                value = customLabel,
                onValueChange = { customLabel = it },
                placeholder = "Label for the chip",
                keyboardType = KeyboardType.Text,
            )

            Spacer(Modifier.height(4.dp))

            OhdButton(
                label = "Add custom",
                onClick = {
                    val l = customLabel.trim()
                    if (l.isNotEmpty()) {
                        scope.launch { sheetState.hide() }.invokeOnCompletion {
                            onDismiss()
                            onPick(l, "custom", "activity")
                        }
                    }
                },
                variant = OhdButtonVariant.Ghost,
                enabled = customLabel.trim().isNotEmpty(),
                modifier = Modifier.fillMaxWidth(),
            )

            Spacer(Modifier.height(8.dp))
        }
    }
}

@Composable
private fun PresetRow(preset: FavouritePreset, onClick: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(48.dp)
            .clickable { onClick() }
            .padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Box(
            modifier = Modifier.size(22.dp),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                imageVector = preset.icon,
                contentDescription = null,
                tint = OhdColors.Red,
                modifier = Modifier.size(20.dp),
            )
        }
        Text(
            text = preset.label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 15.sp,
            color = OhdColors.Ink,
        )
    }
}

// =============================================================================
// HomeFavourite shape — persisted to Auth as a JSON array of these.
// =============================================================================

/**
 * One entry in the `home_favourites_v1` JSON array. The encoding is
 * hand-rolled (one `{ "label": …, "kind": …, "icon": … }` object per row)
 * so we don't pull in a JSON library just for this.
 */
data class HomeFavourite(
    val label: String,
    val kind: String,
    val iconKey: String,
) {
    /**
     * Resolve [iconKey] into an [ImageVector] at render time. New keys can
     * be added without breaking existing rows — fall back to `Activity` for
     * anything unrecognised.
     */
    fun resolveIcon(): ImageVector = when (iconKey) {
        "heart_pulse" -> OhdIcons.HeartPulse
        "droplets" -> OhdIcons.Droplets
        "activity" -> OhdIcons.Activity
        "thermometer" -> OhdIcons.Thermometer
        "favorite" -> OhdIcons.HeartPulse
        else -> OhdIcons.Activity
    }
}

// -----------------------------------------------------------------------------
// JSON codec — hand-rolled.
// -----------------------------------------------------------------------------

/**
 * Decode the persisted blob into a list. Tolerant: returns an empty list
 * on parse failure so a single bad entry doesn't strand the user with no
 * favourites on home.
 *
 * Expected shape: `[{"label":"…","kind":"…","icon":"…"}, …]`.
 */
fun decodeHomeFavourites(json: String?): List<HomeFavourite> {
    if (json.isNullOrBlank()) return emptyList()
    return runCatching {
        val arr = org.json.JSONArray(json)
        (0 until arr.length()).mapNotNull { i ->
            val o = arr.optJSONObject(i) ?: return@mapNotNull null
            HomeFavourite(
                label = o.optString("label", "").ifBlank { return@mapNotNull null },
                kind = o.optString("kind", "custom"),
                iconKey = o.optString("icon", "activity"),
            )
        }
    }.getOrDefault(emptyList())
}

/**
 * Append [favourite] to the list and return the re-encoded JSON. Same
 * `{ label, kind, icon }` shape per row.
 */
fun encodeHomeFavourites(list: List<HomeFavourite>): String {
    val arr = org.json.JSONArray()
    list.forEach { fav ->
        arr.put(
            org.json.JSONObject().apply {
                put("label", fav.label)
                put("kind", fav.kind)
                put("icon", fav.iconKey)
            },
        )
    }
    return arr.toString()
}
