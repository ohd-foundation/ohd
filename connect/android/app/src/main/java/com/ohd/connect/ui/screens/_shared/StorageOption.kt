package com.ohd.connect.ui.screens._shared

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.RetentionLimits
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Storage option choices — shared by the onboarding `OnboardingStorageScreen`
 * (first-run, no top bar) and the in-Settings `StorageSettingsScreen`. The
 * underlying value is the same regardless of how the user reached the picker.
 */
enum class StorageOption {
    OnDevice,
    OhdCloud,
    SelfHosted,
    ProviderHosted,
}

/**
 * Visual model for one card on the storage chooser.
 *
 * Both the onboarding flow and the in-Settings variant render the same four
 * cards, so the shared model lives here.
 */
data class StorageOptionDisplay(
    val option: StorageOption,
    val icon: ImageVector,
    val title: String,
    val description: String,
)

/** Default ordered list of options shown to the user. */
val DefaultStorageOptions: List<StorageOptionDisplay> = listOf(
    StorageOptionDisplay(
        option = StorageOption.OnDevice,
        icon = OhdIcons.Smartphone,
        title = "On this device",
        description = "Stored locally. No account, no network.",
    ),
    StorageOptionDisplay(
        option = StorageOption.OhdCloud,
        icon = OhdIcons.Cloud,
        title = "OHD Cloud",
        description = "Synced across devices. Requires network.",
    ),
    StorageOptionDisplay(
        option = StorageOption.SelfHosted,
        icon = OhdIcons.Server,
        title = "Self-hosted",
        description = "Your own server. Full control.",
    ),
    StorageOptionDisplay(
        option = StorageOption.ProviderHosted,
        icon = OhdIcons.Building2,
        title = "Provider hosted",
        description = "Via your insurer, employer or clinic.",
    ),
)

/**
 * One option card. Selected cards have a 1.5 dp `ohd-ink` border and an
 * expanded panel below the header (controlled by [expandedContent]).
 *
 * The card matches Pencil §4.4 — `eKtkU.png` ground-truth.
 */
@Composable
fun StorageOptionCard(
    display: StorageOptionDisplay,
    selected: Boolean,
    onSelect: () -> Unit,
    modifier: Modifier = Modifier,
    expandedContent: (@Composable () -> Unit)? = null,
) {
    val shape = RoundedCornerShape(12.dp)
    val borderColor = if (selected) OhdColors.Ink else OhdColors.Line
    val borderWidth = if (selected) 1.5.dp else 1.dp

    Column(
        modifier = modifier
            .fillMaxWidth()
            .background(OhdColors.Bg, shape)
            .border(BorderStroke(borderWidth, borderColor), shape)
            .clickable { onSelect() },
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // 20 dp ellipse radio.
            Box(
                modifier = Modifier
                    .size(20.dp)
                    .let {
                        if (selected) {
                            it.background(OhdColors.Ink, CircleShape)
                        } else {
                            it
                                .background(OhdColors.Bg, CircleShape)
                                .border(BorderStroke(1.5.dp, OhdColors.Line), CircleShape)
                        }
                    },
            )

            Icon(
                imageVector = display.icon,
                contentDescription = null,
                tint = if (selected) OhdColors.Ink else OhdColors.Muted,
                modifier = Modifier.size(22.dp),
            )

            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    text = display.title,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 15.sp,
                    color = OhdColors.Ink,
                )
                Text(
                    text = display.description,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
            }
        }

        if (selected && expandedContent != null) {
            // Expanded panel with elevated background.
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(OhdColors.BgElevated)
                    .padding(start = 16.dp, end = 16.dp, top = 0.dp, bottom = 16.dp),
            ) {
                expandedContent()
            }
        }
    }
}

/**
 * Default expanded panel for the "On this device" option. Used by both the
 * onboarding flow and the in-Settings variant unless they need their own
 * content.
 *
 * The chip label reflects the live [retention] state (e.g. "5 GB · 2
 * years ▾"), and tapping it invokes [onClickLimit] which the caller
 * uses to open the retention dialog.
 */
@Composable
fun OnDeviceExpandedPanel(
    retention: RetentionLimits = RetentionLimits(),
    onClickLimit: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            text = "Data is saved as a single file on your device. The file grows " +
                "as you log more entries — typically a few MB per year. You can " +
                "set a retention limit below.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            lineHeight = 18.sp,
            color = OhdColors.Muted,
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(10.dp, Alignment.End),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "Keep data for",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 13.sp,
                color = OhdColors.Ink,
            )
            // Chip "<label> ▾". Tap opens the retention dialog.
            Box(
                modifier = Modifier
                    .background(OhdColors.Bg, RoundedCornerShape(4.dp))
                    .border(BorderStroke(1.dp, OhdColors.Line), RoundedCornerShape(4.dp))
                    .clickable { onClickLimit() }
                    .padding(horizontal = 12.dp, vertical = 6.dp),
            ) {
                Text(
                    text = "${formatRetentionLimits(retention)} ▾",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    color = OhdColors.Ink,
                )
            }
        }
    }
}
