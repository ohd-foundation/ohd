package com.ohd.emergency.data

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * State-machine unit tests for [CaseVault]. Covers the six transitions
 * the break-glass flow can take, plus the queue lifecycle that the
 * persistent store hooks into.
 *
 * No Robolectric / no app context — these are pure JVM tests against
 * the singleton's StateFlow surface.
 */
class CaseVaultLifecycleTest {

    @After
    fun tearDown() {
        CaseVault.clear()
    }

    @Test
    fun startWaiting_sets_waiting_state() {
        CaseVault.startWaiting(
            patientBeaconId = "rdv1",
            operatorLabel = "EMS",
            responderLabel = "Officer Novák",
            timeoutSeconds = 30,
            patientAllowOnTimeout = true,
        )
        assertTrue(CaseVault.breakGlass.value is CaseVault.BreakGlassState.Waiting)
        val s = CaseVault.breakGlass.value as CaseVault.BreakGlassState.Waiting
        assertEquals("Officer Novák", s.responderLabel)
        assertEquals(30, s.timeoutSeconds)
    }

    @Test
    fun grantApproved_opens_active_case() {
        CaseVault.grantApproved(
            patientBeaconId = "rdv1",
            patientLabel = "Patient A",
            caseUlid = "01CASEABC",
            grantToken = "ohdg_test",
            autoGranted = false,
        )
        val active = CaseVault.activeCase.value
        assertEquals("01CASEABC", active?.caseUlid)
        assertEquals("Patient A", active?.patientLabel)
        assertEquals(false, active?.autoGranted)
        assertTrue(CaseVault.breakGlass.value is CaseVault.BreakGlassState.Granted)
    }

    @Test
    fun enqueueIntervention_appends_to_queue_and_flips_status() {
        CaseVault.grantApproved(
            patientBeaconId = "rdv1",
            patientLabel = "Patient",
            caseUlid = "01CASE",
        )
        val w = CaseVault.enqueueIntervention(
            kind = CaseVault.InterventionKind.Vital,
            summary = "HR 112",
            payload = CaseVault.InterventionPayload.Vital("vital.hr", 112.0, "bpm"),
        )
        assertEquals(1, CaseVault.queuedWrites.value.size)
        assertEquals(CaseVault.SyncStatus.Queued, CaseVault.syncStatus.value)
        assertEquals(w.localUlid, CaseVault.queuedWrites.value.first().localUlid)
    }

    @Test
    fun markFlushed_drops_specific_write() {
        CaseVault.grantApproved(
            patientBeaconId = "rdv1",
            patientLabel = "Patient",
            caseUlid = "01CASE",
        )
        val w1 = CaseVault.enqueueIntervention(
            kind = CaseVault.InterventionKind.Vital,
            summary = "A",
            payload = CaseVault.InterventionPayload.Vital("vital.hr", 1.0, "bpm"),
        )
        val w2 = CaseVault.enqueueIntervention(
            kind = CaseVault.InterventionKind.Note,
            summary = "B",
            payload = CaseVault.InterventionPayload.Note("note"),
        )
        CaseVault.markFlushed(w1.localUlid)
        assertEquals(1, CaseVault.queuedWrites.value.size)
        assertEquals(w2.localUlid, CaseVault.queuedWrites.value.first().localUlid)
        // syncStatus stays Queued because there's still a pending write.
        assertEquals(CaseVault.SyncStatus.Queued, CaseVault.syncStatus.value)

        CaseVault.markFlushed(w2.localUlid)
        // Now empty; flips to Synced.
        assertEquals(0, CaseVault.queuedWrites.value.size)
        assertEquals(CaseVault.SyncStatus.Synced, CaseVault.syncStatus.value)
    }

    @Test
    fun clear_resets_everything() {
        CaseVault.grantApproved(
            patientBeaconId = "rdv1",
            patientLabel = "Patient",
            caseUlid = "01CASE",
        )
        CaseVault.enqueueIntervention(
            kind = CaseVault.InterventionKind.Note,
            summary = "n",
            payload = CaseVault.InterventionPayload.Note("text"),
        )
        CaseVault.clear()
        assertEquals(null, CaseVault.activeCase.value)
        assertEquals(0, CaseVault.queuedWrites.value.size)
        assertTrue(CaseVault.breakGlass.value is CaseVault.BreakGlassState.Idle)
    }

    @Test
    fun bleScanner_distance_buckets_match_thresholds() {
        // approximateDistanceFromRssi is the heuristic the real BLE
        // scanner uses; it lives in BleScanner.kt, called from the
        // RealBleScanner. Sanity-check the buckets.
        assertEquals(ApproximateDistance.VeryClose, approximateDistanceFromRssi(-30))
        assertEquals(ApproximateDistance.VeryClose, approximateDistanceFromRssi(-55))
        assertEquals(ApproximateDistance.Close, approximateDistanceFromRssi(-70))
        assertEquals(ApproximateDistance.Nearby, approximateDistanceFromRssi(-85))
        assertEquals(ApproximateDistance.Far, approximateDistanceFromRssi(-95))
    }
}
