package com.ohd.connect.ui.screens

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import android.widget.Toast
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import com.ohd.connect.BuildConfig
import com.ohd.connect.data.AuditEntry
import com.ohd.connect.data.AuditFilter
import com.ohd.connect.data.Auth
import com.ohd.connect.data.EmergencyConfig
import com.ohd.connect.data.GrantTokenStore
import com.ohd.connect.data.QrEncoder
import com.ohd.connect.data.ShareKind
import com.ohd.connect.data.ShareLink
import com.ohd.connect.data.ShareResponders
import com.ohd.connect.data.ShareRow
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.data.toCreateInput
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Share detail — the screen behind a [SharesScreen] row.
 *
 * Implements `cord/spec/data-link.md` §"Connect UI": full scope (event types,
 * channels, sensitivity classes, time window), per-share audit, remote-access
 * status, and actions (edit scope, re-issue link, revoke).
 *
 * Two variants behind one screen:
 *  - **Grant-backed share** — scope card, share-link artifact + QR, audit,
 *    re-issue / revoke actions.
 *  - **Emergency share** ([ShareKind.Emergency]) — carries everything a
 *    normal share does *plus* the break-glass extras (approval timeout,
 *    default-on-timeout, trusted authorities, history window) and a BLE
 *    proximity-beacon affordance, designed so the future beacon toggle has a
 *    place to live.
 *
 * @param shareId the row id — a grant ULID, or [ShareRow.EMERGENCY_SHARE_ID].
 * @param onEditEmergency routes to the full [EmergencySettingsScreen] for the
 *        complete break-glass control set.
 */
@Composable
fun ShareDetailScreen(
    shareId: String,
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onEditEmergency: () -> Unit,
    onToast: (String) -> Unit = {},
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val isEmergency = shareId == ShareRow.EMERGENCY_SHARE_ID

    var share by remember { mutableStateOf<ShareRow?>(null) }
    var audit by remember { mutableStateOf<List<AuditEntry>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    var refreshTick by remember { mutableStateOf(0) }
    var showRevoke by remember { mutableStateOf(false) }
    var reissuedToken by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(refreshTick) {
        // getEmergencyConfig / getGrant / auditQuery are blocking network RPCs
        // against remote storage — run them off the main thread. Snapshot-state
        // assignments below are thread-safe.
        withContext(Dispatchers.IO) {
            if (isEmergency) {
                val cfg = StorageRepository.getEmergencyConfig()
                    .getOrDefault(EmergencyConfig())
                share = ShareRow.emergency(cfg)
            } else {
                StorageRepository.getGrant(shareId)
                    .onSuccess { g ->
                        if (g == null) {
                            error = "Share not found."
                        } else {
                            share = ShareRow.fromGrant(g)
                            error = null
                        }
                    }
                    .onFailure { error = "Couldn't load share: ${it.message}" }
                // Per-share audit — grant-actor audit rows. The uniffi audit
                // filter scopes to actor type; finer per-grant scoping lands with
                // the OHDC `grant_ulid` filter field.
                StorageRepository.auditQuery(
                    AuditFilter(grantUlid = shareId, opKindsIn = emptyList(), limit = 50),
                ).onSuccess { audit = it }
            }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = if (isEmergency) "Emergency share" else "Share", onBack = onBack)

        val s = share
        if (s == null) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text(
                    text = error ?: "Loading…",
                    color = if (error != null) {
                        MaterialTheme.colorScheme.error
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                )
            }
            return
        }

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            HeaderCard(s)

            if (isEmergency) {
                EmergencyExtrasCard(onEditEmergency = onEditEmergency)
                BleBeaconCard()
            } else {
                ScopeCard(s)
                ShareLinkCard(
                    share = s,
                    reissuedToken = reissuedToken,
                    onReissue = {
                        // Re-issue mints a fresh grant token by re-creating a
                        // grant with the same scope. (Storage has no token
                        // rotation RPC; a new grant is the honest way to hand
                        // a grantee a fresh artifact.) The relay rendezvous
                        // registration is a later phase — see
                        // ActivateRemoteAccessCard.
                        val g = s.grant
                        if (g == null) {
                            onToast("Nothing to re-issue.")
                        } else {
                            scope.launch(Dispatchers.IO) {
                                val result = StorageRepository.createGrant(g.toCreateInput())
                                withContext(Dispatchers.Main) {
                                    result.fold(
                                        onSuccess = {
                                            // CreateGrant mints a fresh grant
                                            // row and a fresh bearer for it —
                                            // persist the new bearer keyed by
                                            // the new grant's ULID so a later
                                            // visit to share-detail still
                                            // produces a working link.
                                            GrantTokenStore.save(ctx, it.grantUlid, it.token)
                                            reissuedToken = it.token
                                            onToast("Re-issued share link below.")
                                            refreshTick++
                                        },
                                        onFailure = { e ->
                                            onToast("Re-issue failed: ${e.message}")
                                        },
                                    )
                                }
                            }
                        }
                    },
                )
                // Cloud-direct vs on-device relay path. Storage already on
                // the public internet → there's no local relay tunnel to
                // host; ship a cloud-direct link CORD can paste into its
                // "Add connection" flow. Otherwise the historic relay
                // activation flow.
                if (StorageRepository.isRemoteMode()) {
                    CloudShareCard(
                        // Prefer the just-minted reissue token, fall back to
                        // the persisted bearer from create-time. Final empty
                        // string surfaces the "needs re-issue" state in the
                        // share card rather than encoding the grant ULID as a
                        // (broken) bearer.
                        grantToken = reissuedToken
                            ?: s.grant?.ulid?.let { GrantTokenStore.load(ctx, it) }
                            ?: "",
                        storageUrl = Auth.loadStorageUrl(
                            ctx,
                            StorageRepository.activeMode().name,
                        ) ?: BuildConfig.OHD_CLOUD_STORAGE_URL,
                    )
                } else {
                    ActivateRemoteAccessCard(
                        shareId = shareId,
                        shareLabel = s.label,
                        // Prefer the just-minted reissue token, fall back to
                        // the persisted bearer from create-time. Final empty
                        // string surfaces the "needs re-issue" state in the
                        // share card rather than encoding the grant ULID as a
                        // (broken) bearer.
                        grantToken = reissuedToken
                            ?: s.grant?.ulid?.let { GrantTokenStore.load(ctx, it) }
                            ?: "",
                        onToast = onToast,
                    )
                }
            }

            AuditCard(isEmergency = isEmergency, audit = audit)

            // ---- Actions ----
            if (!isEmergency) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(
                        onClick = { onToast("Edit scope opens in a later build.") },
                        modifier = Modifier.weight(1f),
                    ) { Text("Edit scope") }
                    if (!s.revoked) {
                        Button(
                            onClick = { showRevoke = true },
                            colors = ButtonDefaults.buttonColors(
                                containerColor = MaterialTheme.colorScheme.error,
                                contentColor = MaterialTheme.colorScheme.onError,
                            ),
                            modifier = Modifier.weight(1f),
                        ) { Text("Revoke") }
                    }
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }

    if (showRevoke) {
        AlertDialog(
            onDismissRequest = { showRevoke = false },
            title = { Text("Revoke this share?") },
            text = {
                Text(
                    "Revoking is permanent — the grantee loses access immediately " +
                        "and the share cannot be resumed. To pause a share " +
                        "temporarily, use the toggle on the Shares list instead.",
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    scope.launch(Dispatchers.IO) {
                        StorageRepository.revokeGrant(shareId, null)
                        withContext(Dispatchers.Main) {
                            showRevoke = false
                            refreshTick++
                            onToast("Share revoked.")
                        }
                    }
                }) { Text("Revoke") }
            },
            dismissButton = {
                TextButton(onClick = { showRevoke = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun HeaderCard(s: ShareRow) {
    val nowMs = System.currentTimeMillis()
    DetailCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(modifier = Modifier.weight(1f)) {
                Text(s.label, style = MaterialTheme.typography.titleLarge)
                Spacer(Modifier.height(4.dp))
                val state = when {
                    s.revoked -> "Revoked"
                    !s.enabled -> "Paused"
                    s.expired(nowMs) -> "Expired"
                    else -> "Active"
                }
                Text(
                    text = state,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            AssistChip(
                onClick = {},
                label = { Text(s.kind.label, style = MaterialTheme.typography.labelSmall) },
                colors = AssistChipDefaults.assistChipColors(
                    containerColor = MaterialTheme.colorScheme.primaryContainer,
                ),
            )
        }
        if (s.kind != ShareKind.Emergency) {
            Spacer(Modifier.height(8.dp))
            HorizontalDivider()
            Spacer(Modifier.height(8.dp))
            LabelledRow("Created", fmtDate(s.createdAtMs))
            LabelledRow("Expires", s.expiresAtMs?.let { fmtDate(it) } ?: "never")
            LabelledRow(
                "Last access",
                s.lastUsedMs?.let { fmtRelative(it) } ?: "never",
            )
        }
    }
}

@Composable
private fun ScopeCard(s: ShareRow) {
    val g = s.grant ?: return
    DetailCard {
        Text("Scope", style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(8.dp))
        SectionLabel("Read — event types")
        Text(
            g.readEventTypes.ifEmpty { listOf("(default)") }.joinToString(", "),
            style = MaterialTheme.typography.bodySmall,
        )
        Spacer(Modifier.height(6.dp))
        SectionLabel("Write — event types")
        Text(
            g.writeEventTypes.ifEmpty { listOf("(none — read-only)") }.joinToString(", "),
            style = MaterialTheme.typography.bodySmall,
        )
        if (g.deniedSensitivityClasses.isNotEmpty()) {
            Spacer(Modifier.height(6.dp))
            SectionLabel("Denied sensitivity classes")
            Text(
                g.deniedSensitivityClasses.joinToString(", "),
                style = MaterialTheme.typography.bodySmall,
            )
        }
        Spacer(Modifier.height(6.dp))
        SectionLabel("Approval")
        Text(
            "${g.approvalMode} · default ${g.defaultAction}",
            style = MaterialTheme.typography.bodySmall,
        )
        Spacer(Modifier.height(6.dp))
        SectionLabel("Time window")
        Text(
            "Until " + (g.expiresAtMs?.let { fmtDate(it) } ?: "indefinite"),
            style = MaterialTheme.typography.bodySmall,
        )
    }
}

@Composable
private fun ShareLinkCard(
    share: ShareRow,
    reissuedToken: String?,
    onReissue: () -> Unit,
) {
    // Until the relay-activation phase lands there is no per-share rendezvous;
    // the artifact is the in-person form carrying the grant token. Use the
    // grant ULID as the visible artifact stand-in when no minted token is at
    // hand (the live token is shown once on creation / re-issue).
    val token = reissuedToken ?: share.grant?.ulid ?: ""
    val link = remember(token) { ShareLink.forInPersonGrant(token) }
    DetailCard {
        Text("Share link", style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(4.dp))
        Text(
            "Hand this to the grantee — in person, by tap, or as a QR scan. " +
                "For asynchronous access, activate remote access below.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        OutlinedTextField(
            value = link.canonicalUrl(),
            onValueChange = {},
            readOnly = true,
            label = { Text("ohd:// link") },
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(12.dp))
        QrImage(content = link.canonicalUrl())
        Spacer(Modifier.height(12.dp))
        CopyLinkButton(link.canonicalUrl())
        Spacer(Modifier.height(8.dp))
        OutlinedButton(onClick = onReissue, modifier = Modifier.fillMaxWidth()) {
            Text("Re-issue link")
        }
    }
}

/** A full-width button that copies the share link to the clipboard. */
@Composable
private fun CopyLinkButton(url: String) {
    val clipboard = LocalClipboardManager.current
    val context = LocalContext.current
    OutlinedButton(
        onClick = {
            clipboard.setText(AnnotatedString(url))
            Toast.makeText(context, "Link copied", Toast.LENGTH_SHORT).show()
        },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Text("Copy link")
    }
}

/** Renders a [QrEncoder] matrix onto a Canvas. */
@Composable
private fun QrImage(content: String) {
    val matrix = remember(content) {
        runCatching { QrEncoder.encode(content) }.getOrNull()
    }
    if (matrix == null) {
        Text(
            "QR unavailable for this content.",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        return
    }
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 24.dp),
        contentAlignment = Alignment.Center,
    ) {
        Canvas(
            modifier = Modifier
                .size(220.dp)
                .aspectRatio(1f)
                .background(Color.White)
                .padding(8.dp),
        ) {
            val n = matrix.size
            val cell = size.minDimension / n
            for (y in 0 until n) {
                for (x in 0 until n) {
                    if (matrix.get(x, y)) {
                        drawRect(
                            color = Color.Black,
                            topLeft = Offset(x * cell, y * cell),
                            size = Size(cell, cell),
                        )
                    }
                }
            }
        }
    }
}

/**
 * Remote-access activation — the live CORD data-link Phase 4d surface.
 *
 * Activating registers a per-share relay rendezvous (real
 * `POST /v1/register`), starts the background share responder (relay tunnel
 * + inner-TLS server + share-scoped MCP), and renders the real
 * `ohd://share/...` link + QR built from the rendezvous + SPKI pin.
 */
@Composable
private fun ActivateRemoteAccessCard(
    shareId: String,
    shareLabel: String,
    grantToken: String,
    onToast: (String) -> Unit,
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()

    var binding by remember(shareId) {
        mutableStateOf(ShareResponders.binding(ctx, shareId))
    }
    var busy by remember { mutableStateOf(false) }
    // Relay host the user can override — defaults to OHD's relay. A clinic
    // running its own relay puts its host here; CORD accepts any relay host
    // from the share link (`cord/spec/data-link.md` §"Activating remote
    // access" step 1).
    var relayHost by remember { mutableStateOf(ShareLink.DEFAULT_RELAY_HOST) }

    DetailCard {
        Text("Remote access", style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(4.dp))
        Text(
            "By default a share is in-person only. Activating remote access " +
                "registers a per-share relay rendezvous so the grantee can " +
                "reach your data asynchronously through OHD Relay.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))

        val b = binding
        if (b == null) {
            OutlinedTextField(
                value = relayHost,
                onValueChange = { relayHost = it.trim() },
                singleLine = true,
                label = { Text("Relay host") },
                supportingText = {
                    Text(
                        "Default is OHD Relay. Enter a custom host if your " +
                            "clinic runs its own.",
                    )
                },
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(8.dp))
            OutlinedButton(
                onClick = {
                    if (busy) return@OutlinedButton
                    val host = relayHost.trim()
                    if (host.isEmpty()) {
                        onToast("Enter a relay host.")
                        return@OutlinedButton
                    }
                    busy = true
                    scope.launch {
                        val result = withContext(Dispatchers.IO) {
                            ShareResponders.activate(
                                ctx = ctx,
                                grantUlid = shareId,
                                shareLabel = shareLabel,
                                relayOrigin = ShareResponders.relayOriginForHost(host),
                                relayTunnelUrl = ShareResponders.relayTunnelUrlForHost(host),
                            )
                        }
                        busy = false
                        result.fold(
                            onSuccess = {
                                binding = it
                                onToast("Remote access activated.")
                            },
                            onFailure = { e ->
                                onToast("Activation failed: ${e.message}")
                            },
                        )
                    }
                },
                enabled = !busy,
                modifier = Modifier.fillMaxWidth(),
            ) { Text(if (busy) "Activating…" else "Activate remote access") }
        } else {
            // The real share link — rendezvous + grant token + SPKI pin.
            // The relay host travels in the link so CORD knows which relay
            // to reach; it's the host the user activated against.
            val link = remember(b, grantToken) {
                ShareLink(
                    rendezvousId = b.rendezvousId,
                    token = grantToken,
                    pinSpki = b.spkiPin,
                    relayHost = ShareResponders.hostForRelayOrigin(b.relayOrigin),
                )
            }
            val statusLine = if (ShareResponders.isActive(shareId)) {
                "Responder running — the relay tunnel is open."
            } else {
                "Responder will start when the app next opens this storage."
            }
            Text(
                statusLine,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(
                value = link.canonicalUrl(),
                onValueChange = {},
                readOnly = true,
                label = { Text("ohd://share link") },
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(12.dp))
            QrImage(content = link.canonicalUrl())
            Spacer(Modifier.height(12.dp))
            CopyLinkButton(link.canonicalUrl())
            Spacer(Modifier.height(8.dp))
            OutlinedButton(
                onClick = {
                    if (busy) return@OutlinedButton
                    busy = true
                    scope.launch {
                        withContext(Dispatchers.IO) {
                            ShareResponders.deactivate(ctx, shareId)
                        }
                        busy = false
                        binding = null
                        onToast("Remote access disabled.")
                    }
                },
                enabled = !busy,
                modifier = Modifier.fillMaxWidth(),
            ) { Text("Disable remote access") }
        }
    }
}

/**
 * Cloud-direct share — the third surface in the share flow that bridges
 * the gap when the user's storage is already on the public internet
 * (OHD Cloud or self-hosted with a public Caddy cert). No local relay
 * tunnel to host; CORD reads `storage.ohd.dev` (or whichever URL) over
 * standard HTTPS using just the grant token as the bearer. The link
 * format is what `cord/crates/cord-server/src/share.rs` parses as the
 * `ParsedShare::Cloud` variant.
 */
@Composable
private fun CloudShareCard(
    grantToken: String,
    storageUrl: String,
) {
    val link = remember(grantToken, storageUrl) {
        ShareLink(
            rendezvousId = null,
            token = grantToken,
            pinSpki = null,
            relayHost = null,
            cloudEndpoint = storageUrl,
        )
    }
    DetailCard {
        Text("Cloud share", style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(4.dp))
        Text(
            "Your storage already lives on the public internet, so no relay " +
                "tunnel is needed — the grantee reaches it directly with the " +
                "grant token below. Paste the link into CORD's Add connection " +
                "field, or scan the QR.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        Text(
            text = "Storage: $storageUrl",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        OutlinedTextField(
            value = link.canonicalUrl(),
            onValueChange = {},
            readOnly = true,
            label = { Text("ohd://share link") },
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(12.dp))
        QrImage(content = link.canonicalUrl())
        Spacer(Modifier.height(12.dp))
        CopyLinkButton(link.canonicalUrl())
    }
}

@Composable
private fun EmergencyExtrasCard(onEditEmergency: () -> Unit) {
    // getEmergencyConfig is a blocking network RPC against remote storage —
    // load it off the main thread rather than inside `remember {}` (which runs
    // during composition on the main dispatcher and would freeze the UI).
    var cfg by remember { mutableStateOf(EmergencyConfig()) }
    LaunchedEffect(Unit) {
        cfg = withContext(Dispatchers.IO) {
            StorageRepository.getEmergencyConfig().getOrDefault(EmergencyConfig())
        }
    }
    DetailCard {
        Text("Break-glass settings", style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(4.dp))
        Text(
            "Emergency is a share with break-glass semantics — first " +
                "responders can request access, gated by your approval or a " +
                "timeout.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        LabelledRow("Feature", if (cfg.featureEnabled) "enabled" else "disabled")
        LabelledRow("Approval timeout", "${cfg.approvalTimeoutSeconds}s")
        LabelledRow(
            "On timeout",
            when (cfg.defaultOnTimeout) {
                EmergencyConfig.DefaultAction.ALLOW -> "allow access"
                EmergencyConfig.DefaultAction.REFUSE -> "refuse access"
            },
        )
        LabelledRow("History window", "${cfg.historyWindowHours}h")
        LabelledRow("Trusted authorities", "${cfg.trustRoots.size} configured")
        Spacer(Modifier.height(10.dp))
        Button(onClick = onEditEmergency, modifier = Modifier.fillMaxWidth()) {
            Text("Edit break-glass settings")
        }
    }
}

/**
 * BLE proximity-beacon affordance. The beacon lets an in-office responder's
 * device discover this phone (and, later, carry the relay config — see
 * `cord/spec/data-link.md` §"BLE-assisted config"). The toggle itself is a
 * disabled stub so the future capability has a designed home.
 */
@Composable
private fun BleBeaconCard() {
    DetailCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(modifier = Modifier.weight(1f)) {
                Text("Proximity beacon", style = MaterialTheme.typography.titleMedium)
                Spacer(Modifier.height(2.dp))
                Text(
                    "Broadcasts a low-power Bluetooth signal so an in-office " +
                        "responder's device can find your OHD record. No health " +
                        "data leaves the phone over Bluetooth.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Switch(checked = false, onCheckedChange = {}, enabled = false)
        }
        Text(
            "Beacon support ships in a later release.",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(top = 6.dp),
        )
    }
}

@Composable
private fun AuditCard(isEmergency: Boolean, audit: List<AuditEntry>) {
    DetailCard {
        Text("Access history", style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(4.dp))
        if (isEmergency) {
            Text(
                "Emergency access events appear in the global Audit log under " +
                    "Profile & Access.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            return@DetailCard
        }
        if (audit.isEmpty()) {
            Text(
                "No access recorded yet.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        } else {
            audit.take(20).forEach { e ->
                Spacer(Modifier.height(6.dp))
                Text(
                    "${fmtRelative(e.tsMs)} · ${e.opName}" +
                        (e.rowsReturned?.let { " · $it rows" } ?: ""),
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
    }
}

// ---- small reusable bits ---------------------------------------------------

@Composable
private fun DetailCard(content: @Composable () -> Unit) {
    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
            content()
        }
    }
}

@Composable
private fun SectionLabel(text: String) {
    Text(text, style = MaterialTheme.typography.labelMedium)
}

@Composable
private fun LabelledRow(label: String, value: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 2.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(
            label,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(value, style = MaterialTheme.typography.bodySmall)
    }
}
