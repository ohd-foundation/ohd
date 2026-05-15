package com.ohd.connect.data

/**
 * Shares — the user-facing data-sharing model.
 *
 * Implements the Connect side of `cord/spec/data-link.md` ("Shares — the
 * user-facing model"). A **Share** is the single concept the user manages:
 * *"this party may see this slice of my data."* It unifies two things that
 * are separate lower in the stack — OHD grants and the emergency break-glass
 * profile — into one list and one mental model.
 *
 * A Share = a grant (scope: read/write rules, sensitivity classes, time
 * window, expiry) plus an optional remote-access binding (a relay rendezvous
 * + connection artifact). The relay-activation backend is a later phase; this
 * file models the share itself and the share-link artifact.
 */

/** Kind of share — drives the type chip and which detail screen variant shows. */
enum class ShareKind(val label: String) {
    /** A clinician grant. */
    Doctor("doctor"),

    /** A family-member / spouse grant. */
    Family("family"),

    /** A researcher / study grant — aggregation-only, notes stripped. */
    Researcher("researcher"),

    /** An automated agent (CORD or another MCP consumer). */
    Agent("agent"),

    /**
     * The emergency break-glass profile, modelled as a first-class share.
     * Pinned to the top of the list, non-deletable; its detail screen
     * carries the break-glass extras on top of the normal share info.
     */
    Emergency("emergency");

    companion object {
        /**
         * Classify a grant into a share kind from its `granteeKind` plus a
         * couple of heuristics. `emergency_authority` → [Emergency];
         * `researcher` → [Researcher]; `app` / `service` → [Agent]; a grant
         * with no write rules and notify-on-access on reads as [Family];
         * everything else is a clinician [Doctor].
         */
        fun classify(grant: GrantSummary): ShareKind = when {
            grant.granteeKind == "emergency_authority" -> Emergency
            grant.granteeKind == "researcher" -> Researcher
            grant.granteeKind == "app" || grant.granteeKind == "service" -> Agent
            grant.writeEventTypes.isEmpty() && grant.granteeKind == "user" &&
                grant.readEventTypes.size <= 4 -> Family
            else -> Doctor
        }
    }
}

/**
 * One row in the Shares list. A thin projection over [GrantSummary] (plus a
 * synthetic row for the emergency profile) carrying exactly what the list and
 * detail screens render.
 */
data class ShareRow(
    /**
     * Stable id. For grant-backed shares this is the grant ULID. For the
     * synthetic emergency row it is the sentinel [EMERGENCY_SHARE_ID].
     */
    val id: String,
    val label: String,
    val kind: ShareKind,
    /** `true` only for the pinned, non-deletable emergency row. */
    val pinned: Boolean,
    /** Underlying grant, when this share is grant-backed. Null for emergency. */
    val grant: GrantSummary?,
    val createdAtMs: Long,
    val expiresAtMs: Long?,
    val revokedAtMs: Long?,
    val suspendedAtMs: Long?,
    val lastUsedMs: Long?,
    val useCount: Long,
) {
    /** A suspended OR revoked share is "off"; otherwise "on". */
    val enabled: Boolean get() = suspendedAtMs == null && revokedAtMs == null

    val revoked: Boolean get() = revokedAtMs != null

    fun expired(nowMs: Long): Boolean = expiresAtMs != null && expiresAtMs < nowMs

    companion object {
        /** Sentinel id for the synthetic, always-present emergency share row. */
        const val EMERGENCY_SHARE_ID = "emergency"

        /** Project a grant into a share row. */
        fun fromGrant(g: GrantSummary): ShareRow = ShareRow(
            id = g.ulid,
            label = g.granteeLabel,
            kind = ShareKind.classify(g),
            pinned = false,
            grant = g,
            createdAtMs = g.createdAtMs,
            expiresAtMs = g.expiresAtMs,
            revokedAtMs = g.revokedAtMs,
            suspendedAtMs = g.suspendedAtMs,
            lastUsedMs = g.lastUsedMs,
            useCount = g.useCount,
        )

        /**
         * The synthetic emergency share row. `enabled` mirrors the emergency
         * feature master switch so the list toggle and the break-glass
         * feature toggle stay in lock-step.
         */
        fun emergency(cfg: EmergencyConfig): ShareRow = ShareRow(
            id = EMERGENCY_SHARE_ID,
            label = "Emergency",
            kind = ShareKind.Emergency,
            pinned = true,
            grant = null,
            createdAtMs = 0L,
            expiresAtMs = null,
            revokedAtMs = null,
            // Feature-disabled reads as "suspended" so the row toggle is off.
            suspendedAtMs = if (cfg.featureEnabled) null else 1L,
            lastUsedMs = null,
            useCount = 0L,
        )
    }
}

/**
 * The share-link artifact a user hands to a grantee.
 *
 * Canonical form (`cord/spec/data-link.md` §"The share link artifact"):
 *
 * ```
 * ohd://share/<rendezvous_id>?token=<ohdg_…>&pin=<spki>&relay=<host>
 * ```
 *
 * The relay-activation backend (registering a real rendezvous) is a later
 * phase. Until a share has remote access activated there is no `rendezvous_id`
 * and no `pin`; [forInPersonGrant] produces the in-person form that carries
 * just the grant token, and [hasRemoteAccess] is false.
 */
data class ShareLink(
    /** Per-share rendezvous on the relay; null until remote access is activated. */
    val rendezvousId: String?,
    /** The `ohdg_…` grant token — the bearer credential. */
    val token: String,
    /** SHA-256 of the storage identity-key SPKI; null until remote access. */
    val pinSpki: String?,
    /** Relay host; null for in-person-only shares. */
    val relayHost: String?,
) {
    val hasRemoteAccess: Boolean get() = rendezvousId != null

    /** The canonical `ohd://…` URL — what the QR code encodes. */
    fun canonicalUrl(): String =
        if (hasRemoteAccess) {
            "ohd://share/$rendezvousId?token=$token" +
                (pinSpki?.let { "&pin=$it" } ?: "") +
                (relayHost?.let { "&relay=$it" } ?: "")
        } else {
            // In-person form: no rendezvous yet, carry the bare grant token.
            "ohd://grant/$token"
        }

    /**
     * `https://` mirror — for browsers / contexts with no custom-scheme
     * handler. Credentials ride the URL **fragment** so they are never sent
     * to the relay's web server. Null when remote access isn't activated
     * (no relay host to mirror against).
     */
    fun httpsMirrorUrl(): String? =
        if (hasRemoteAccess && relayHost != null) {
            "https://$relayHost/share/$rendezvousId#token=$token" +
                (pinSpki?.let { "&pin=$it" } ?: "")
        } else {
            null
        }

    companion object {
        /** Default OHD relay host (`cord/spec/data-link.md`). */
        const val DEFAULT_RELAY_HOST = "relay.ohd.dev"

        /** In-person-only artifact: the grant token, no rendezvous, no pin. */
        fun forInPersonGrant(token: String): ShareLink =
            ShareLink(rendezvousId = null, token = token, pinSpki = null, relayHost = null)
    }
}

/**
 * Reconstruct a [CreateGrantInput] from a [GrantSummary] — used by the
 * share-detail "re-issue link" action. Storage exposes no token-rotation RPC,
 * so re-issuing means creating a fresh grant with the same scope. The
 * GrantSummary carries every field that survives the uniffi `GrantDto`
 * projection (write rules are not separately surfaced on the DTO, so a
 * re-issued grant is read-only when the original had no readable write rules).
 */
fun GrantSummary.toCreateInput(): CreateGrantInput = CreateGrantInput(
    granteeLabel = granteeLabel,
    granteeKind = granteeKind,
    purpose = purpose,
    approvalMode = approvalMode,
    defaultAction = defaultAction,
    expiresAtMs = expiresAtMs,
    readEventTypes = readEventTypes,
    writeEventTypes = writeEventTypes,
    autoApproveEventTypes = emptyList(),
    denySensitivityClasses = deniedSensitivityClasses,
)
