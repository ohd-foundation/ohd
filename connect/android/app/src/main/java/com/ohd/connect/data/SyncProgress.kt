package com.ohd.connect.data

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/**
 * Process-wide live progress for a Health Connect → OHD sync run.
 *
 * [syncFromHealthConnect] publishes a running count of events persisted so a
 * UI open during the sync (the Health Connect settings screen, or a background
 * periodic run observed live) can show "N events synced" instead of just a
 * spinner. A single in-memory [StateFlow] is enough — there is at most one
 * sync at a time (the worker is unique work; the manual button is guarded).
 */
object SyncProgress {
    data class State(
        /** True while a sync run is in flight. */
        val running: Boolean = false,
        /** Events persisted so far in the current (or last) run. */
        val synced: Int = 0,
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    /** Mark the start of a run; resets the counter to zero. */
    fun begin() {
        _state.value = State(running = true, synced = 0)
    }

    /** Publish the latest cumulative synced-event count for this run. */
    fun report(synced: Int) {
        _state.update { it.copy(synced = synced) }
    }

    /** Mark the run finished; the final [State.synced] count is retained. */
    fun end() {
        _state.update { it.copy(running = false) }
    }
}
