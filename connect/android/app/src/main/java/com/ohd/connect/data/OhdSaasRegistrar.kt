package com.ohd.connect.data

import android.content.Context
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * Glue that pushes the locally-minted [OhdAccount] up to `api.ohd.dev`
 * without blocking the caller. The token returned by `register` is
 * persisted via [OhdSaasTokenStore] so subsequent calls (`me`, `linkOidc`,
 * …) can use it.
 *
 * Designed to be safe in the offline / api-not-deployed case:
 *  - the local account works without a server round-trip,
 *  - failures are logged and dropped,
 *  - the next app start re-attempts iff no token is on file.
 */
object OhdSaasRegistrar {

    private const val TAG = "OhdSaasRegistrar"

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /**
     * Kicks off a one-shot registration. Returns immediately; the caller
     * is not awaited.
     */
    fun fireAndForget(ctx: Context, account: OhdAccount) {
        if (OhdSaasTokenStore.load(ctx) != null) return
        scope.launch {
            val recovery = account.recoveryCode.lines
                .indices
                .joinToString("\n") { account.recoveryCode.formatRow(it) }
            OhdSaasClient.register(account.profileUlid, recovery).fold(
                onSuccess = {
                    OhdSaasTokenStore.save(ctx, it.accessToken)
                    Log.i(TAG, "registered ${it.profileUlid}")
                },
                onFailure = { e ->
                    Log.w(TAG, "register failed (offline?); local account stands", e)
                },
            )
        }
    }
}
