package com.ohd.connect.data

import android.content.Context
import android.util.Log
import org.json.JSONObject
import uniffi.ohd_storage.RemoteShareDto
import uniffi.ohd_storage.ShareResponderHandle

/**
 * In-process registry of running share responders.
 *
 * Implements the "keep the responder running while the share has remote
 * access enabled" half of `cord/spec/data-link.md` "Activating remote
 * access" (step 3 — "Connect opens/maintains the tunnel").
 *
 * One [ShareResponderHandle] is kept per share with remote access on. The
 * handle owns a background tokio runtime in the Rust core that maintains
 * the relay tunnel and answers scoped MCP; stopping the handle deregisters
 * the tunnel and tears that runtime down.
 *
 * Lifecycle:
 *  - **Activate** — [ShareDetailScreen]'s "Activate remote access" calls
 *    [activate]: register the rendezvous, persist the binding, start the
 *    responder.
 *  - **Resume on launch** — [resumeAll] walks every persisted binding and
 *    re-starts its responder, so a share left remote-enabled comes back up
 *    after an app restart.
 *  - **Deactivate** — [deactivate] stops the responder and clears the
 *    persisted binding.
 *
 * The registry is process-scoped; it does not survive a process death on
 * its own — [resumeAll] (called from `MainActivity`) is what restores it.
 */
object ShareResponders {

    private const val TAG = "OhdConnect.ShareResponders"

    /**
     * Default OHD relay QUIC-tunnel endpoint — `host:port` of the relay's
     * `--quic-tunnel-listen`. The registration origin is the HTTPS origin;
     * the tunnel endpoint is the raw-QUIC one (default port 9001).
     */
    const val DEFAULT_RELAY_ORIGIN = "https://relay.ohd.dev"
    const val DEFAULT_RELAY_TUNNEL_URL = "relay.ohd.dev:9001"

    /** Live handles, keyed by grant ULID. */
    private val running = mutableMapOf<String, ShareResponderHandle>()

    /**
     * A share's persisted remote-access binding — everything needed to
     * rebuild the share link and resume the responder after a restart.
     * Serialized to JSON in [Auth.saveRemoteShare].
     */
    data class Binding(
        val rendezvousId: String,
        val rendezvousUrl: String,
        val longLivedCredential: String,
        val spkiPin: String,
        val relayOrigin: String,
        val relayTunnelUrl: String,
    ) {
        fun toJson(): String = JSONObject().apply {
            put("rendezvous_id", rendezvousId)
            put("rendezvous_url", rendezvousUrl)
            put("long_lived_credential", longLivedCredential)
            put("spki_pin", spkiPin)
            put("relay_origin", relayOrigin)
            put("relay_tunnel_url", relayTunnelUrl)
        }.toString()

        fun toDto(): RemoteShareDto = RemoteShareDto(
            rendezvousId = rendezvousId,
            rendezvousUrl = rendezvousUrl,
            longLivedCredential = longLivedCredential,
            spkiPinB64url = spkiPin,
        )

        companion object {
            fun fromJson(json: String): Binding {
                val o = JSONObject(json)
                return Binding(
                    rendezvousId = o.getString("rendezvous_id"),
                    rendezvousUrl = o.getString("rendezvous_url"),
                    longLivedCredential = o.getString("long_lived_credential"),
                    spkiPin = o.getString("spki_pin"),
                    relayOrigin = o.getString("relay_origin"),
                    relayTunnelUrl = o.getString("relay_tunnel_url"),
                )
            }
        }
    }

    /** True iff a responder is currently running for this share. */
    fun isActive(grantUlid: String): Boolean = running[grantUlid]?.isRunning() == true

    /** The persisted binding for a share, or null when remote access is off. */
    fun binding(ctx: Context, grantUlid: String): Binding? =
        Auth.getRemoteShare(ctx, grantUlid)?.let {
            runCatching { Binding.fromJson(it) }.getOrNull()
        }

    /**
     * Activate remote access for a share: register a per-share relay
     * rendezvous, persist the binding, and start the background responder.
     *
     * Returns the [Binding] on success — the caller builds the
     * `ohd://share/...` link from it. The relay registration is a real
     * network call; a failure surfaces as a `Result.failure`.
     *
     * @param relayOrigin the relay HTTPS origin (default [DEFAULT_RELAY_ORIGIN]).
     * @param relayTunnelUrl the relay QUIC tunnel `host:port`
     *        (default [DEFAULT_RELAY_TUNNEL_URL]).
     * @param allowInsecureDev accept any relay QUIC cert — dev / local relay.
     */
    fun activate(
        ctx: Context,
        grantUlid: String,
        shareLabel: String?,
        relayOrigin: String = DEFAULT_RELAY_ORIGIN,
        relayTunnelUrl: String = DEFAULT_RELAY_TUNNEL_URL,
        allowInsecureDev: Boolean = false,
    ): Result<Binding> {
        val identityKey = StorageRepository.storageIdentityKey()

        return StorageRepository.registerRemoteShare(
            grantUlid = grantUlid,
            relayOrigin = relayOrigin,
            identityKeyHex = identityKey,
            shareLabel = shareLabel,
        ).mapCatching { dto ->
            val binding = Binding(
                rendezvousId = dto.rendezvousId,
                rendezvousUrl = dto.rendezvousUrl,
                longLivedCredential = dto.longLivedCredential,
                spkiPin = dto.spkiPinB64url,
                relayOrigin = relayOrigin,
                relayTunnelUrl = relayTunnelUrl,
            )
            Auth.saveRemoteShare(ctx, grantUlid, binding.toJson())
            startResponder(grantUlid, binding, identityKey, allowInsecureDev)
            binding
        }
    }

    /**
     * Disable remote access for a share: stop the responder and clear the
     * persisted binding. Idempotent. (Deregistering the rendezvous on the
     * relay happens server-side when the credential goes unused; the local
     * teardown is what matters for the phone.)
     */
    fun deactivate(ctx: Context, grantUlid: String) {
        running.remove(grantUlid)?.stop()
        Auth.saveRemoteShare(ctx, grantUlid, null)
    }

    /**
     * Resume every share left with remote access enabled — called once from
     * `MainActivity` after the storage handle is open. A share whose
     * responder is already running is skipped.
     */
    fun resumeAll(ctx: Context, grantUlids: List<String>) {
        val identityKey = runCatching { StorageRepository.storageIdentityKey() }
            .getOrNull() ?: return
        for (ulid in grantUlids) {
            if (isActive(ulid)) continue
            val binding = binding(ctx, ulid) ?: continue
            runCatching { startResponder(ulid, binding, identityKey, allowInsecureDev = false) }
                .onFailure { Log.w(TAG, "resume responder failed for $ulid", it) }
        }
    }

    /** Stop every running responder — called on storage close / sign-out. */
    fun stopAll() {
        running.values.forEach { it.stop() }
        running.clear()
    }

    private fun startResponder(
        grantUlid: String,
        binding: Binding,
        identityKeyHex: String,
        allowInsecureDev: Boolean,
    ) {
        // Replace any stale handle for this share.
        running.remove(grantUlid)?.stop()
        StorageRepository.startShareResponder(
            grantUlid = grantUlid,
            share = binding.toDto(),
            relayTunnelUrl = binding.relayTunnelUrl,
            identityKeyHex = identityKeyHex,
            allowInsecureDev = allowInsecureDev,
        ).onSuccess { handle ->
            running[grantUlid] = handle
            Log.i(TAG, "share responder running for $grantUlid")
        }.onFailure {
            Log.w(TAG, "start responder failed for $grantUlid", it)
            throw it
        }
    }
}
