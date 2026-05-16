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

    /** Default raw-QUIC tunnel port — the relay's `--quic-tunnel-listen`. */
    const val DEFAULT_RELAY_TUNNEL_PORT = 9001

    /**
     * Derive the relay's HTTPS registration origin from a user-entered
     * relay host. A bare host (`relay.example.com`) becomes
     * `https://relay.example.com`; a value the user already prefixed with a
     * scheme is taken as-is. Supports the "custom relay" path in
     * `cord/spec/data-link.md` §"Activating remote access".
     */
    fun relayOriginForHost(host: String): String {
        val h = host.trim().removeSuffix("/")
        return if (h.startsWith("http://") || h.startsWith("https://")) h
        else "https://$h"
    }

    /**
     * Derive the relay's raw-QUIC tunnel `host:port` from a relay host.
     * The host portion is the registration host with any scheme / path
     * stripped; the port defaults to [DEFAULT_RELAY_TUNNEL_PORT] unless the
     * user already supplied one.
     */
    fun relayTunnelUrlForHost(host: String): String {
        val bare = hostForRelayOrigin(host)
        return if (bare.contains(':')) bare else "$bare:$DEFAULT_RELAY_TUNNEL_PORT"
    }

    /**
     * The bare relay host (no scheme, no path, no trailing slash) for a
     * relay origin or host string — what travels in the `relay=` parameter
     * of the `ohd://share/...` link.
     */
    fun hostForRelayOrigin(origin: String): String =
        origin.trim()
            .removePrefix("https://")
            .removePrefix("http://")
            .substringBefore('/')

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

    /**
     * Number of share responders currently running — i.e. the count of
     * remote connections the relay can reach. [ShareResponderService] uses
     * this both for the persistent notification's "N connection(s)" text and
     * to decide whether the foreground service still has work to do.
     */
    fun activeCount(): Int = running.values.count { it.isRunning() }

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
            // Hand the responder a durable host: the foreground service
            // keeps the process (and its tunnel) alive while the app is
            // backgrounded. Idempotent — re-triggers the resume path if the
            // service is already running.
            ShareResponderService.start(ctx)
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
        // When the last remote share is turned off the foreground service
        // has nothing left to host — stop it so its notification disappears.
        ShareResponderService.stopIfIdle(ctx)
    }

    /**
     * Resume every share the user left with remote access enabled.
     *
     * The set of shares to resume is derived from the **persisted bindings**
     * ([Auth.listRemoteShareGrantUlids]) — a binding exists iff remote access
     * is on. It must NOT be derived by filtering `listGrants(...)`: the Shares
     * UI lists with `includeRevoked = true` while the resume path used
     * `includeRevoked = false`, so any share whose grant fell outside that
     * filter was activatable yet never reconnected after an app restart.
     *
     * A share whose responder is already running is skipped. A binding whose
     * grant no longer resolves fails [startResponder] gracefully (logged).
     */
    fun resumeAll(ctx: Context) {
        val identityKey = runCatching { StorageRepository.storageIdentityKey() }
            .getOrNull() ?: return
        for (ulid in Auth.listRemoteShareGrantUlids(ctx)) {
            if (isActive(ulid)) continue
            val binding = binding(ctx, ulid) ?: continue
            runCatching { startResponder(ulid, binding, identityKey, allowInsecureDev = false) }
                .onFailure { Log.w(TAG, "resume responder failed for $ulid", it) }
        }
        // CRITICAL: must NOT start ShareResponderService here.
        // ShareResponderService.onStartCommand() itself calls resumeAll(), so
        // starting the service from inside resumeAll() is an infinite restart
        // loop: start → onStartCommand → resumeAll → start → … It pegs the
        // main thread, floods the FGS notification, and thrashes the relay
        // tunnel. Callers that want the durable host start the service
        // themselves — see [activate] and `MainActivity`'s cold-start path.
    }

    /**
     * Handle a relay push-wake (`WAKE_REQUEST`, `relay-protocol.md` §frame
     * `0x09`) — invoked from [com.ohd.connect.data.RelayWakeService] when an
     * FCM data-only message with `category = "tunnel_wake"` arrives.
     *
     * The relay sends this when a consumer (CORD) attaches at a rendezvous
     * whose phone-side tunnel is currently down — typically because the
     * process was killed or the device dozed. We must re-establish the
     * share responders so the relay's bounded wait sees a live tunnel and
     * can complete the consumer's attach.
     *
     * Because the wake can land on a cold process, storage may not be open.
     * We open it with the persisted stub key (the same path `MainActivity`
     * uses on cold start) before resuming. Best-effort: a failure to open
     * storage is logged, not thrown — the relay falls the attach back
     * cleanly on its own timeout.
     *
     * Safe to call from a background thread; does no UI work.
     */
    fun wake(ctx: Context) {
        StorageRepository.init(ctx)
        if (!StorageRepository.isOpen()) {
            val opened = when {
                StorageRepository.isInitialised() ->
                    StorageRepository.open("00".repeat(32))
                else -> Result.failure(IllegalStateException("storage not initialised"))
            }
            opened.onFailure {
                Log.w(TAG, "push-wake: could not open storage; cannot resume responders", it)
                return
            }
        }
        val remoteShares = Auth.listRemoteShareGrantUlids(ctx)
        if (remoteShares.isEmpty()) {
            Log.i(TAG, "push-wake: no remote shares to resume")
            return
        }
        Log.i(TAG, "push-wake: resuming responders for ${remoteShares.size} share(s)")
        resumeAll(ctx)
        // A push-woken responder should also get the durable foreground host.
        // Safe here — `wake` is not on the onStartCommand → resumeAll path, so
        // this does not re-enter the restart loop guarded against in resumeAll.
        if (activeCount() > 0) ShareResponderService.start(ctx)
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
