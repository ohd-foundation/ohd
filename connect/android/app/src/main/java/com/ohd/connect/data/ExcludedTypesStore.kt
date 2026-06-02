package com.ohd.connect.data

import android.content.Context

/**
 * Persists the set of event-type names the user has long-press-hidden on the
 * History screen. Not sensitive (just presentation), so a plain
 * SharedPreferences file — no need for the encrypted store.
 *
 * One entry per event type as a CSV string under the single key
 * `excluded_csv`. CSV instead of a `Set<String>` because String-set
 * serialization in SharedPreferences has a documented platform quirk
 * (mutation-while-iterating returns a different View on different OEMs).
 *
 * Heart rate is the obvious one to hide on a HC-synced phone — thousands
 * of samples/day saturate the list. Future history-screen redesign will
 * supersede this with the per-family visibility model in
 * `spec/docs/future-implementations/history-and-aggregates.md`; until then
 * this is the lightweight escape hatch.
 */
object ExcludedTypesStore {

    private const val FILE = "ohd_history_prefs"
    private const val KEY = "excluded_csv"

    fun load(ctx: Context): Set<String> {
        val raw = prefs(ctx).getString(KEY, null) ?: return emptySet()
        return raw.split(',').mapNotNull { it.trim().takeIf { s -> s.isNotEmpty() } }.toSet()
    }

    fun save(ctx: Context, types: Set<String>) {
        val csv = types.joinToString(",")
        prefs(ctx).edit().putString(KEY, csv).apply()
    }

    private fun prefs(ctx: Context) =
        ctx.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)
}
