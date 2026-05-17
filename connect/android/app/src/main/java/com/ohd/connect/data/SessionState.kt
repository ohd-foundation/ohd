package com.ohd.connect.data

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/**
 * Phase 4 — app-wide remote-session state holder.
 *
 * A single observable flag, [reloginNeeded], that flips to `true` the moment
 * any remote storage call fails with a terminal [RemoteAuthException]
 * ("session expired / revoked — sign in again"). [StorageRepository.withBackend]
 * folds every backend `Result.failure` carrying a [RemoteAuthException] into
 * [reportFailure], so the Compose shell can observe one flag rather than
 * every ~39 call site having to special-case the auth error.
 *
 * The shell ([com.ohd.connect.MainActivity]) renders a "Your session expired —
 * sign in again" banner / dialog when this is set and routes the user back to
 * the storage sign-in. After a successful re-sign-in (or a switch to
 * on-device) the flag is cleared via [clear].
 *
 * Deliberately tiny: it is *not* a general event bus. It carries the single
 * fact the UI needs to react to and nothing else.
 */
object SessionState {

    /**
     * `true` once a remote call has hit a terminal auth error. Compose
     * observes this directly (it is a `mutableStateOf`-backed property), so a
     * background-thread write triggers recomposition of any reader.
     */
    var reloginNeeded: Boolean by mutableStateOf(false)
        private set

    /**
     * Inspect a failed [Result] and raise [reloginNeeded] when the cause is a
     * terminal [RemoteAuthException]. Transient [RemoteStorageException]s
     * (network blips) are ignored — those are surfaced inline per-screen.
     *
     * Safe to call from any thread; `mutableStateOf` writes are thread-safe
     * for the snapshot system.
     */
    fun reportFailure(error: Throwable) {
        if (error is RemoteAuthException) {
            reloginNeeded = true
        }
    }

    /**
     * Clear the flag — called after the user has either re-signed-in
     * successfully or switched storage back to on-device.
     */
    fun clear() {
        reloginNeeded = false
    }
}
