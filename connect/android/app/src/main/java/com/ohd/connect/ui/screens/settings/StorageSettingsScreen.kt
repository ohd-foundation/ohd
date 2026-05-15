package com.ohd.connect.ui.screens.settings

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.Auth
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/** Storage option per spec §4.4 — four exclusive choices. */
enum class StorageOption(
    internal val title: String,
    internal val desc: String,
) {
    OnDevice(
        title = "On this device",
        desc = "Stored locally. No account, no network.",
    ),
    OhdCloud(
        title = "OHD Cloud",
        desc = "Synced across devices. Requires network.",
    ),
    SelfHosted(
        title = "Self-hosted",
        desc = "Your own server. Full control.",
    ),
    ProviderHosted(
        title = "Provider hosted",
        desc = "Via your insurer, employer or clinic.",
    ),
}

/**
 * Storage & Data settings — reuses Configure Storage layout from Pencil
 * `eKtkU` (spec §4.4) inside the Settings stack.
 *
 * Top bar (back + "Storage & Data"), then heading + subtitle + 4 option
 * cards (radio + icon + title/desc, expanded panel inside the selected
 * option) + notice card + Continue button.
 *
 * State is hoisted on [selectedOption] / [onSelect]. The Continue button
 * just calls [onContinue] — actual storage path-picking still lives in
 * `SetupScreen` for the onboarding flow; this screen is the visual
 * replacement that gets reached from Settings → Storage.
 */
@Composable
fun StorageSettingsScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onContinue: () -> Unit,
    selectedOption: StorageOption = StorageOption.OnDevice,
    onSelect: (StorageOption) -> Unit = {},
    onToast: (String) -> Unit = {},
) {
    val ctx = LocalContext.current
    // Bug #4: pre-select from the persisted onboarding choice rather than
    // hard-coded OnDevice. The persisted value is the enum `name`; map back
    // to this screen's StorageOption enum (parallel to `_shared.StorageOption`).
    val persistedName = Auth.loadStorageOption(ctx, defaultName = StorageOption.OnDevice.name)
    val persistedOption =
        StorageOption.entries.firstOrNull { it.name == persistedName } ?: selectedOption
    var localSelected by remember(persistedName) { mutableStateOf(persistedOption) }
    val effectiveSelected = localSelected
    val select: (StorageOption) -> Unit = {
        // Switching away from the currently-active option is a v1.x feature.
        // For now: show the "coming soon" notice the user already sees in
        // onboarding, and snap the radio back to the persisted choice.
        if (it != persistedOption) {
            onToast("Switching storage is coming soon — your data stays on this device.")
            localSelected = persistedOption
        } else {
            localSelected = it
        }
        onSelect(localSelected)
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Storage & Data", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            // 1. Heading.
            Text(
                text = "Where should OHD store your data?",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W300,
                fontSize = 22.sp,
                lineHeight = 29.sp,
                color = OhdColors.Ink,
                modifier = Modifier.fillMaxWidth(),
            )

            // 2. Subtitle.
            Text(
                text = "You can change this at any time. Your data is always your property regardless of where it lives.",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 13.sp,
                lineHeight = 19.5.sp,
                color = OhdColors.Muted,
            )

            // 3. Four option cards.
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                StorageOptionCard(
                    option = StorageOption.OnDevice,
                    icon = OhdIcons.Smartphone,
                    selected = effectiveSelected == StorageOption.OnDevice,
                    onClick = { select(StorageOption.OnDevice) },
                )
                StorageOptionCard(
                    option = StorageOption.OhdCloud,
                    icon = OhdIcons.Cloud,
                    selected = effectiveSelected == StorageOption.OhdCloud,
                    onClick = { select(StorageOption.OhdCloud) },
                )
                StorageOptionCard(
                    option = StorageOption.SelfHosted,
                    icon = OhdIcons.Server,
                    selected = effectiveSelected == StorageOption.SelfHosted,
                    onClick = { select(StorageOption.SelfHosted) },
                )
                StorageOptionCard(
                    option = StorageOption.ProviderHosted,
                    icon = OhdIcons.Building2,
                    selected = effectiveSelected == StorageOption.ProviderHosted,
                    onClick = { select(StorageOption.ProviderHosted) },
                )
            }

            // 4. Notice card.
            NoticeCard()

            // 5. Continue button. The Continue path doesn't migrate data —
            // selection snap-back already happened in [select]. We surface a
            // last reassurance toast if the user picked a non-active option,
            // mirroring the onboarding "coming soon" notice.
            OhdButton(
                label = "Continue",
                onClick = {
                    if (localSelected != persistedOption) {
                        onToast("Switching storage is coming soon — your data stays on this device.")
                    }
                    onContinue()
                },
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

/**
 * One option card per spec §4.4.
 *
 * Corner `radius-lg`, fill `ohd-bg`, 1 dp `ohd-line` border. Selected state
 * has 1.5 dp `ohd-ink` border AND a body panel below the header row with
 * the per-option explainer + retention chip.
 *
 * Header row layout: 20 dp ellipse radio + 22 dp Lucide icon + (title /
 * desc) text block, gap 12.
 */
@Composable
private fun StorageOptionCard(
    option: StorageOption,
    icon: ImageVector,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val shape = RoundedCornerShape(12.dp)
    val borderColor = if (selected) OhdColors.Ink else OhdColors.Line
    val borderWidth = if (selected) 1.5.dp else 1.dp

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(OhdColors.Bg, shape)
            .border(BorderStroke(borderWidth, borderColor), shape)
            .clickable { onClick() },
    ) {
        // Header row.
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            RadioDot(selected = selected)
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = if (selected) OhdColors.Ink else OhdColors.Muted,
                modifier = Modifier.size(22.dp),
            )
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = option.title,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 15.sp,
                    color = OhdColors.Ink,
                )
                Text(
                    text = option.desc,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
            }
        }

        if (selected) {
            ExpandedPanel(option = option)
        }
    }
}

/**
 * 20 dp ellipse radio button.
 *
 * Selected: filled `ohd-ink` with a small white centre dot. Unselected:
 * empty with 1.5 dp `ohd-line` border.
 */
@Composable
private fun RadioDot(selected: Boolean) {
    Box(
        modifier = Modifier
            .size(20.dp)
            .background(
                color = if (selected) OhdColors.Ink else OhdColors.Bg,
                shape = CircleShape,
            )
            .border(
                width = 1.5.dp,
                color = if (selected) OhdColors.Ink else OhdColors.Line,
                shape = CircleShape,
            ),
        contentAlignment = Alignment.Center,
    ) {
        if (selected) {
            Box(
                modifier = Modifier
                    .size(7.dp)
                    .background(OhdColors.White, CircleShape),
            )
        }
    }
}

/**
 * Per-option explainer + retention chip, shown only on the selected card.
 *
 * Spec §4.4: vertical, padding `[t=0, b=16, h=16]`, fill `ohd-bg-elevated`,
 * gap 12. Explainer text + management row with "Keep data for" + "Forever ▾"
 * chip.
 */
@Composable
private fun ExpandedPanel(option: StorageOption) {
    val explainer = when (option) {
        StorageOption.OnDevice ->
            "Data is saved as a single file on your device. The file grows as you log more entries — typically a few MB per year. You can set a retention limit below."
        StorageOption.OhdCloud ->
            "Synced through OHD's hosted service so you can move between devices. End-to-end encrypted; OHD staff cannot read your data."
        StorageOption.SelfHosted ->
            "Point OHD at your own server (an OHDC-compatible endpoint). You hold the keys and run the infrastructure."
        StorageOption.ProviderHosted ->
            "Your insurer, employer or clinic operates the storage on your behalf. You retain access and can export at any time."
    }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(OhdColors.BgElevated)
            .padding(start = 16.dp, end = 16.dp, top = 0.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            text = explainer,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            lineHeight = 18.sp,
            color = OhdColors.Muted,
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Box(modifier = Modifier.weight(1f))
            Text(
                text = "Keep data for",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 13.sp,
                color = OhdColors.Ink,
            )
            RetentionChip()
        }
    }
}

/** "Forever ▾" chip — 1 dp `ohd-line` border, padding `[v=6, h=12]`, radius-sm. */
@Composable
private fun RetentionChip() {
    val shape = RoundedCornerShape(4.dp)
    Row(
        modifier = Modifier
            .background(OhdColors.Bg, shape)
            .border(BorderStroke(1.dp, OhdColors.Line), shape)
            .padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(
            text = "Forever",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Ink,
        )
        Icon(
            imageVector = OhdIcons.ChevronDown,
            contentDescription = null,
            tint = OhdColors.Ink,
            modifier = Modifier.size(14.dp),
        )
    }
}

/**
 * Notice card — corner `radius-md`, fill `ohd-bg-elevated`, padding 12,
 * gap 8: 16 dp `lucide:shield-check` `ohd-muted` + explainer text.
 */
@Composable
private fun NoticeCard() {
    val shape = RoundedCornerShape(8.dp)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(OhdColors.BgElevated, shape)
            .padding(12.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(
            imageVector = OhdIcons.ShieldCheck,
            contentDescription = null,
            tint = OhdColors.Muted,
            modifier = Modifier
                .size(16.dp)
                .padding(top = 2.dp),
        )
        Text(
            text = "Switching storage later migrates all your data. Nothing is lost. Your data is always exportable as an encrypted OHD archive — easily converted to JSONL. Full format spec in the docs (link coming).",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            lineHeight = 18.sp,
            color = OhdColors.Muted,
            modifier = Modifier.weight(1f),
        )
    }
}
