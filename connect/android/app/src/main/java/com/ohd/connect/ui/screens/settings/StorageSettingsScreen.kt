package com.ohd.connect.ui.screens.settings

import androidx.activity.ComponentActivity
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
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
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
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.screens._shared.StorageOption
import com.ohd.connect.ui.screens._shared.StorageSignInPanel
import com.ohd.connect.ui.screens._shared.StorageSignInResult
import com.ohd.connect.ui.screens._shared.rememberStorageAuthLauncher
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Card title/description for a [StorageOption] — spec §4.4.
 *
 * These strings used to live on a second, settings-private `StorageOption`
 * enum that paralleled `_shared.StorageOption`. The two enums shared the same
 * four `name` values (so `Auth.{save,load}StorageOption` worked across both),
 * but kept drifting. The enum is now unified on `_shared.StorageOption`; the
 * presentation strings live here as a local mapping.
 */
private val StorageOption.title: String
    get() = when (this) {
        StorageOption.OnDevice -> "On this device"
        StorageOption.OhdCloud -> "OHD Cloud"
        StorageOption.SelfHosted -> "Self-hosted"
        StorageOption.ProviderHosted -> "Provider hosted"
    }

private val StorageOption.desc: String
    get() = when (this) {
        StorageOption.OnDevice -> "Stored locally. No account, no network."
        StorageOption.OhdCloud -> "Synced across devices. Requires network."
        StorageOption.SelfHosted -> "Your own server. Full control."
        StorageOption.ProviderHosted -> "Via your insurer, employer or clinic."
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
    val activity = ctx as? ComponentActivity
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    // Bug #4: pre-select from the persisted onboarding choice rather than
    // hard-coded OnDevice. The persisted value is the enum `name`; map back
    // to this screen's StorageOption enum (parallel to `_shared.StorageOption`).
    val persistedName = Auth.loadStorageOption(ctx, defaultName = StorageOption.OnDevice.name)
    val persistedOption =
        StorageOption.entries.firstOrNull { it.name == persistedName } ?: selectedOption
    var localSelected by remember(persistedName) { mutableStateOf(persistedOption) }
    val effectiveSelected = localSelected

    // Phase 4 — local SQLCipher stub key, mirroring MainActivity's open path.
    // TODO: real key derivation per spec/encryption.md.
    val localKeyHex = "00".repeat(32)

    // Phase 4 — "Signed in as <identity>" surface. `StorageRepository.identity()`
    // is backend-aware: in remote mode it returns the storage URL + the
    // `whoami` identity (a network call), so resolve it off the main thread
    // and only when the app is actually running against remote storage.
    var remoteMode by remember { mutableStateOf(StorageRepository.isRemoteMode()) }
    var signedInIdentity by remember { mutableStateOf<String?>(null) }
    var signOutInFlight by remember { mutableStateOf(false) }
    LaunchedEffect(remoteMode) {
        signedInIdentity = if (remoteMode) {
            withContext(Dispatchers.IO) {
                runCatching { StorageRepository.identity().userUlid }.getOrNull()
            }
        } else {
            null
        }
    }

    // Phase 2 — picker → OIDC sign-in. Selecting a remote option no longer
    // shows a "coming soon" toast; it expands a sign-in panel. On a
    // successful Custom-Tab return the token + URL + option are persisted
    // and the row shows a "Signed in" state.
    var signedInOption by remember {
        mutableStateOf(
            StorageOption.entries.firstOrNull {
                it != StorageOption.OnDevice && Auth.loadStorageUrl(ctx, it.name) != null
            },
        )
    }
    var signInError by remember { mutableStateOf<String?>(null) }
    var inFlightOption by remember { mutableStateOf<StorageOption?>(null) }
    // Editable URL fields for self/provider-hosted, pre-filled from any
    // persisted URL so a re-sign-in doesn't make the user retype.
    val urlFields = remember {
        mutableStateMapOf<StorageOption, String>().apply {
            StorageOption.entries.forEach { opt ->
                put(opt, Auth.loadStorageUrl(ctx, opt.name).orEmpty())
            }
        }
    }

    val authLauncher = rememberStorageAuthLauncher(
        activity = activity ?: return,
    ) { result ->
        inFlightOption = null
        when (result) {
            is StorageSignInResult.Success -> {
                signInError = null
                signedInOption = result.option
                localSelected = result.option
                onSelect(result.option)
                // Phase 4 — live swap: the OIDC sign-in persisted the option +
                // URL + token, so flip the in-process backend to remote without
                // requiring an app restart.
                scope.launch {
                    val swap = withContext(Dispatchers.IO) {
                        StorageRepository.switchTo(result.option, localKeyHex)
                    }
                    swap
                        .onSuccess {
                            remoteMode = StorageRepository.isRemoteMode()
                            onToast("Signed in to ${result.option.title}.")
                        }
                        .onFailure { e ->
                            signInError = "Switched sign-in but storage failed to open: ${e.message}"
                        }
                }
            }
            is StorageSignInResult.Failure -> {
                signInError = result.message
                // Leave the option as it was — snap back to the last
                // successfully-signed-in / on-device choice.
                localSelected = signedInOption ?: StorageOption.OnDevice
            }
        }
    }

    // Phase 4 — sign out of remote storage and live-swap back to on-device.
    val signOut: () -> Unit = {
        if (!signOutInFlight) {
            signOutInFlight = true
            scope.launch {
                val result = withContext(Dispatchers.IO) {
                    StorageRepository.signOutToLocal(localKeyHex)
                }
                signOutInFlight = false
                result
                    .onSuccess {
                        remoteMode = false
                        signedInOption = null
                        signedInIdentity = null
                        localSelected = StorageOption.OnDevice
                        onSelect(StorageOption.OnDevice)
                        onToast("Signed out — your data is now on this device.")
                    }
                    .onFailure { e ->
                        signInError = "Sign-out failed: ${e.message}"
                    }
            }
        }
    }

    // Phase 4 — "Danger zone" state. Tracks the confirm-dialog visibility
    // and the in-flight delete so the button can be disabled + relabeled
    // while the network call runs.
    var showDeleteDialog by remember { mutableStateOf(false) }
    var deleting by remember { mutableStateOf(false) }

    val select: (StorageOption) -> Unit = { opt ->
        signInError = null
        // Selecting on-device while signed into remote storage IS a sign-out:
        // run the full sign-out + live swap rather than just expanding a card.
        if (opt == StorageOption.OnDevice && remoteMode) {
            localSelected = opt
            signOut()
        } else {
            // On-device (when already local) is available with no login.
            // Remote options just expand the card so the user can run the
            // OIDC sign-in below.
            localSelected = opt
            onSelect(opt)
        }
    }

    val startSignIn: (StorageOption, String) -> Unit = { opt, url ->
        signInError = null
        inFlightOption = opt
        authLauncher.launch(opt, url)
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

            // 2b. Phase 4 — active remote-storage surface. When the app is
            // running against a remote `ohd-storage-server`, show the storage
            // URL + "Signed in as <identity>" + a Sign out action. On-device
            // mode keeps the unchanged four-card display below.
            if (remoteMode) {
                SignedInCard(
                    storageUrl = Auth.loadStorageUrl(ctx, StorageRepository.activeMode().name),
                    identity = signedInIdentity,
                    signOutInFlight = signOutInFlight,
                    onSignOut = signOut,
                )
            }

            // 3. Four option cards.
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                val cardIcons = mapOf(
                    StorageOption.OnDevice to OhdIcons.Smartphone,
                    StorageOption.OhdCloud to OhdIcons.Cloud,
                    StorageOption.SelfHosted to OhdIcons.Server,
                    StorageOption.ProviderHosted to OhdIcons.Building2,
                )
                StorageOption.entries.forEach { opt ->
                    StorageOptionCard(
                        option = opt,
                        icon = cardIcons.getValue(opt),
                        selected = effectiveSelected == opt,
                        onClick = { select(opt) },
                        signedIn = signedInOption == opt,
                        signedInUrl = Auth.loadStorageUrl(ctx, opt.name),
                        inFlight = inFlightOption == opt,
                        signInError = if (effectiveSelected == opt) signInError else null,
                        urlValue = urlFields[opt].orEmpty(),
                        onUrlChange = { urlFields[opt] = it },
                        onSignIn = { url -> startSignIn(opt, url) },
                    )
                }
            }

            // 4. Notice card.
            NoticeCard()

            // 5. Continue button. The Continue path doesn't migrate data —
            // Phase 2 persists the chosen storage option + URL + token only
            // when the OIDC sign-in actually completes. A remote option
            // selected but not yet signed into is not committed, so Continue
            // just leaves whatever was last persisted in place.
            OhdButton(
                label = "Continue",
                onClick = {
                    if (localSelected != StorageOption.OnDevice && signedInOption != localSelected) {
                        onToast("Sign in to ${localSelected.title} to switch storage.")
                    }
                    onContinue()
                },
                modifier = Modifier.fillMaxWidth(),
            )

            // 6. Danger zone — hard-wipe of every event the signed-in identity
            // owns on the remote server. Only meaningful in remote-storage
            // mode; on-device users wipe by uninstalling the app.
            if (remoteMode) {
                OhdSectionHeader(text = "Danger zone")
                OhdButton(
                    label = if (deleting) "Deleting…" else "Delete all my remote data",
                    onClick = { showDeleteDialog = true },
                    modifier = Modifier.fillMaxWidth(),
                    variant = OhdButtonVariant.Destructive,
                    enabled = !deleting,
                )
            }
        }
    }

    if (showDeleteDialog) {
        AlertDialog(
            onDismissRequest = { if (!deleting) showDeleteDialog = false },
            title = { Text("Delete all remote data?") },
            text = {
                Text(
                    "This hard-deletes every event the signed-in identity owns on the remote server. " +
                        "It cannot be undone. Only events and channels are wiped — grants, cases and " +
                        "audit logs are NOT removed.",
                )
            },
            confirmButton = {
                TextButton(
                    enabled = !deleting,
                    colors = ButtonDefaults.textButtonColors(contentColor = OhdColors.Red),
                    onClick = {
                        showDeleteDialog = false
                        deleting = true
                        // Storage call blocks on a remote RPC — off the main
                        // thread, marshal the result back for the toast.
                        scope.launch(Dispatchers.IO) {
                            val res = StorageRepository.deleteRemoteEvents()
                            withContext(Dispatchers.Main) {
                                deleting = false
                                res
                                    .onSuccess { count -> onToast("Deleted $count events") }
                                    .onFailure { e -> onToast("Couldn't delete: ${e.message}") }
                            }
                        }
                    },
                ) { Text("Delete everything") }
            },
            dismissButton = {
                TextButton(
                    enabled = !deleting,
                    onClick = { showDeleteDialog = false },
                ) { Text("Cancel") }
            },
        )
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
    signedIn: Boolean,
    signedInUrl: String?,
    inFlight: Boolean,
    signInError: String?,
    urlValue: String,
    onUrlChange: (String) -> Unit,
    onSignIn: (String) -> Unit,
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
            ExpandedPanel(
                option = option,
                signedIn = signedIn,
                signedInUrl = signedInUrl,
                inFlight = inFlight,
                signInError = signInError,
                urlValue = urlValue,
                onUrlChange = onUrlChange,
                onSignIn = onSignIn,
            )
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
private fun ExpandedPanel(
    option: StorageOption,
    signedIn: Boolean,
    signedInUrl: String?,
    inFlight: Boolean,
    signInError: String?,
    urlValue: String,
    onUrlChange: (String) -> Unit,
    onSignIn: (String) -> Unit,
) {
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
        if (option == StorageOption.OnDevice) {
            // On-device: unchanged retention-chip row.
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
        } else {
            // Remote options: the real OIDC sign-in panel (Phase 2).
            StorageSignInPanel(
                option = option,
                signedIn = signedIn,
                signedInUrl = signedInUrl,
                inFlight = inFlight,
                urlValue = urlValue,
                onUrlChange = onUrlChange,
                onSignIn = onSignIn,
                errorMessage = signInError,
            )
        }
    }
}

/**
 * Phase 4 — "Signed in" surface for the active remote storage backend.
 *
 * Shown above the option cards when [StorageRepository.isRemoteMode] is
 * `true`: the storage server URL, "Signed in as `<identity>`" (the remote
 * `whoami` user ULID), and a "Sign out" action. Sign-out clears the local
 * session, best-effort RP-logs-out of the AS, and live-swaps the app back to
 * on-device storage — no restart.
 */
@Composable
private fun SignedInCard(
    storageUrl: String?,
    identity: String?,
    signOutInFlight: Boolean,
    onSignOut: () -> Unit,
) {
    val shape = RoundedCornerShape(8.dp)
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(OhdColors.BgElevated, shape)
            .border(BorderStroke(1.dp, OhdColors.Line), shape)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(
                imageVector = OhdIcons.Cloud,
                contentDescription = null,
                tint = OhdColors.Success,
                modifier = Modifier.size(18.dp),
            )
            Text(
                text = "Connected to remote storage",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W600,
                fontSize = 14.sp,
                color = OhdColors.Ink,
            )
        }
        Text(
            text = storageUrl ?: "(remote storage)",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            lineHeight = 18.sp,
            color = OhdColors.Muted,
        )
        Text(
            text = "Signed in as ${identity ?: "…"}",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 13.sp,
            color = OhdColors.Ink,
        )
        OhdButton(
            label = if (signOutInFlight) "Signing out…" else "Sign out",
            onClick = onSignOut,
            modifier = Modifier.fillMaxWidth(),
            variant = OhdButtonVariant.Destructive,
            enabled = !signOutInFlight,
        )
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
