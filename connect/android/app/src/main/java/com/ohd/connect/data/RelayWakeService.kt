package com.ohd.connect.data

import android.util.Log
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

/**
 * FCM receiver for the relay's silent push-wake.
 *
 * Implements the phone side of `relay-protocol.md` §frame `0x09`
 * (`WAKE_REQUEST`) and the "Relay tunnel wake-up" row of
 * `connect/spec/notifications.md`:
 *
 * > Relay → push → Connect re-establishes tunnel — Silent (data-only push,
 * > no UI surface).
 *
 * When a consumer (CORD) attaches at a rendezvous whose phone-side tunnel
 * is down, the relay sends a high-priority **data-only** FCM message:
 *
 * ```json
 * { "category": "tunnel_wake", "ref_ulid": "<rendezvous_id>" }
 * ```
 *
 * Because the message carries `data` only (no `notification` key), Android
 * delivers it to [onMessageReceived] even when the app is backgrounded or
 * the process was killed — exactly the case the wake exists for. We resume
 * the share responders ([ShareResponders.wake]) so the relay's bounded
 * wait sees a live tunnel and completes the consumer's attach. No OS
 * notification is shown — the wake is invisible to the user.
 *
 * No PHI is ever in the payload; the `ref_ulid` is an opaque rendezvous id.
 *
 * Registered in `AndroidManifest.xml` with the `MESSAGING_EVENT`
 * intent-filter.
 */
class RelayWakeService : FirebaseMessagingService() {

    /**
     * Wake category the relay's FCM client tags tunnel-wake pushes with —
     * see `relay/src/push/fcm.rs` (`TunnelWakePayload.category`).
     */
    private val wakeCategory = "tunnel_wake"

    override fun onMessageReceived(message: RemoteMessage) {
        val category = message.data["category"]
        if (category != wakeCategory) {
            // Other categories (pending_write, emergency, …) are handled
            // by their own paths once those land; ignore here.
            Log.d(TAG, "ignoring push with category=$category")
            return
        }
        val refUlid = message.data["ref_ulid"]
        Log.i(TAG, "relay push-wake received (ref=$refUlid); resuming responders")
        runCatching { ShareResponders.wake(applicationContext) }
            .onFailure { Log.w(TAG, "push-wake handling failed", it) }
    }

    /**
     * Called when FCM issues / rotates this device's registration token.
     *
     * Threading the token up to the relay (so the relay can push-wake this
     * device) is the `Notify.RegisterDevice` path — not yet built; it needs
     * a live Firebase project. When that lands, this is where the fresh
     * token is forwarded into each remote share's relay registration.
     */
    override fun onNewToken(token: String) {
        Log.i(TAG, "FCM registration token refreshed")
        // TODO(notifications): forward to the relay via Notify.RegisterDevice
        // / RefreshRegistration so push-wake can reach this device.
    }

    private companion object {
        const val TAG = "OhdConnect.RelayWake"
    }
}
