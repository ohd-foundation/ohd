package com.ohd.emergency.data

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.ParcelUuid
import android.util.Log
import androidx.core.content.ContextCompat
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.flow
import java.util.UUID

/**
 * BLE patient discovery — emergency.
 *
 * The OHD beacon's BLE service UUID is an open item per
 * `spec/emergency-trust.md` "Open items":
 *
 *     Concrete BLE service UUID + characteristic IDs are deferred …
 *     Tablet and Connect (patient side) need to agree.
 *
 * Until that lands, [RealBleScanner] uses [PLACEHOLDER_OHD_SERVICE_UUID]
 * — a 16-bit-derived 128-bit UUID under the SIG-allocated short-form
 * range. The canonical OHD UUID is a v0.x deliverable; once pinned in
 * `spec/emergency-trust.md`, replace the constant. The tablet side is
 * one constant change away from a live deployment.
 *
 * Two implementations:
 *  - [MockBleScanner]: emits 3 mock patients in ~3s (used by the v0
 *    demo so the UI works without BLE hardware).
 *  - [RealBleScanner]: scans via Android's [android.bluetooth.le.BluetoothLeScanner]
 *    against the OHD service UUID; parses service-data to extract the
 *    opaque beacon ID.
 *
 * The `beacon_id` returned here is not the patient's storage public key
 * (that would defeat anonymous beacon broadcast). It's the rotating
 * 16-byte opaque ID from the OHD beacon protocol; the operator's relay
 * resolves it to a target patient via the relay-mediated bystander
 * chain at `/emergency/initiate` time.
 */
const val PLACEHOLDER_OHD_SERVICE_UUID = "0000FED0-0000-1000-8000-00805F9B34FB"
data class DiscoveredBeacon(
    /** Opaque beacon identifier broadcast in the BLE service data. 16 bytes hex. */
    val beaconId: String,
    /** The label the patient phone advertises (optional; "Patient" if absent). */
    val displayLabel: String?,
    /** RSSI in dBm. Smaller magnitude = closer; -50 is "right next to me", -90 is "across the street". */
    val rssiDbm: Int,
    /** Best-effort distance estimate from RSSI. UI-only. */
    val approximateDistance: ApproximateDistance,
    /** First-seen timestamp (ms since epoch). */
    val firstSeenAtMs: Long,
    /** Last-seen timestamp (ms since epoch). */
    val lastSeenAtMs: Long,
)

enum class ApproximateDistance {
    /** RSSI ≥ -55: within arm's reach. The patient is the one in front of you. */
    VeryClose,
    /** -55 > RSSI ≥ -75: same room. */
    Close,
    /** -75 > RSSI ≥ -90: same building / nearby vehicle. */
    Nearby,
    /** RSSI < -90: marginal. Beacon is on the edge of detection. */
    Far,
}

interface BleScanner {
    /** Scan for OHD beacons until cancelled. Emits cumulative results. */
    fun scan(): Flow<List<DiscoveredBeacon>>
}

/**
 * Mock implementation: emits a few patients with distance variation,
 * arriving at staggered intervals so the discovery screen can show its
 * live-update behaviour.
 *
 * Three mock patients are returned — enough to exercise the "tap to pick"
 * affordance without burying the user in pretend results. Names follow
 * `screens-emergency.md`'s "Officer Novák" / "EMS Prague Region" idiom.
 */
class MockBleScanner : BleScanner {
    override fun scan(): Flow<List<DiscoveredBeacon>> = flow {
        val now = System.currentTimeMillis()
        val results = mutableListOf<DiscoveredBeacon>()

        // The first patient appears almost immediately (within 600ms).
        delay(600)
        results.add(
            DiscoveredBeacon(
                beaconId = "b3a2:7f04:e1d9:8c15",
                displayLabel = "Patient — apartment 4B",
                rssiDbm = -52,
                approximateDistance = ApproximateDistance.VeryClose,
                firstSeenAtMs = now + 600,
                lastSeenAtMs = now + 600,
            )
        )
        emit(results.toList())

        // A second patient appears at ~1.6s (someone in another room).
        delay(1000)
        results.add(
            DiscoveredBeacon(
                beaconId = "9f44:0123:abcd:55ee",
                displayLabel = "Patient",
                rssiDbm = -71,
                approximateDistance = ApproximateDistance.Close,
                firstSeenAtMs = now + 1600,
                lastSeenAtMs = now + 1600,
            )
        )
        emit(results.toList())

        // A third faint beacon at ~3s (perhaps a person in a vehicle outside).
        delay(1400)
        results.add(
            DiscoveredBeacon(
                beaconId = "11aa:22bb:33cc:44dd",
                displayLabel = null,
                rssiDbm = -88,
                approximateDistance = ApproximateDistance.Nearby,
                firstSeenAtMs = now + 3000,
                lastSeenAtMs = now + 3000,
            )
        )
        emit(results.toList())
    }
}

// =============================================================================
// RealBleScanner — Android BluetoothLeScanner against the OHD service UUID.
// =============================================================================

/**
 * Production BLE scanner. Uses [android.bluetooth.le.BluetoothLeScanner]
 * to listen for OHD beacons and emits the cumulative observed set on
 * each scan result.
 *
 * # Permissions
 *
 * Caller is responsible for runtime-requesting `BLUETOOTH_SCAN` (API 31+)
 * or `ACCESS_FINE_LOCATION` (≤ 30) before constructing this scanner;
 * use [hasBleScanPermission] to gate. The flow emits an empty list and
 * logs a warning when permission is missing — it does not throw, so the
 * UI degrades gracefully (paramedic can fall back to manual entry).
 *
 * # Scan parameters
 *
 * - `SCAN_MODE_LOW_LATENCY`: the paramedic is here-and-now; we want
 *   results within ~1s, not background-trickle scanning. Drains battery
 *   faster but the scan only runs while the discovery screen is visible.
 * - `MATCH_MODE_AGGRESSIVE`: surface even weak beacons. Distance
 *   estimation downstream filters by RSSI threshold for the UI distance
 *   chip; a faint result is still useful (beacon-in-next-room case).
 * - `CALLBACK_TYPE_ALL_MATCHES`: every advertisement, not just first
 *   match — RSSI updates as the paramedic walks closer.
 *
 * # Beacon ID extraction
 *
 * The advertisement's service-data payload (under the OHD service UUID)
 * carries the rotating 16-byte opaque beacon ID per the OHD beacon
 * protocol. Format documented at `spec/emergency-trust.md` "Beacon ID";
 * v0 layout (provisional):
 *
 *     [0..16]  rotating opaque beacon ID (16 bytes)
 *     [16..]   reserved for future extension
 *
 * If the service-data payload is missing or shorter than 16 bytes, we
 * fall back to the BLE device address as a stable-but-not-canonical
 * identifier so the UI can still render the row (degraded mode).
 */
class RealBleScanner(
    private val ctx: Context,
    private val serviceUuid: UUID = UUID.fromString(PLACEHOLDER_OHD_SERVICE_UUID),
) : BleScanner {

    companion object {
        private const val TAG = "OhdEmergency.BleScanner"
        /** Stop scans after this long even if the UI forgets to cancel. */
        const val DEFAULT_SCAN_BUDGET_MS = 30_000L
    }

    @SuppressLint("MissingPermission")
    override fun scan(): Flow<List<DiscoveredBeacon>> = callbackFlow {
        if (!hasBleScanPermission(ctx)) {
            Log.w(TAG, "BLUETOOTH_SCAN / ACCESS_FINE_LOCATION not granted; emitting empty.")
            trySend(emptyList())
            close()
            return@callbackFlow
        }

        val manager = ctx.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
        val adapter: BluetoothAdapter? = manager?.adapter
        if (adapter == null || !adapter.isEnabled) {
            Log.w(TAG, "BluetoothAdapter unavailable or disabled; emitting empty.")
            trySend(emptyList())
            close()
            return@callbackFlow
        }
        val scanner = adapter.bluetoothLeScanner
        if (scanner == null) {
            Log.w(TAG, "bluetoothLeScanner returned null; adapter may be turning off.")
            trySend(emptyList())
            close()
            return@callbackFlow
        }

        val seen = mutableMapOf<String, DiscoveredBeacon>()

        val callback = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                val beacon = mapToBeacon(result) ?: return
                seen[beacon.beaconId] = beacon.copy(
                    firstSeenAtMs = seen[beacon.beaconId]?.firstSeenAtMs ?: beacon.firstSeenAtMs,
                )
                trySend(seen.values.sortedByDescending { it.rssiDbm })
            }

            override fun onBatchScanResults(results: MutableList<ScanResult>) {
                results.forEach { onScanResult(SCAN_RESULT_TYPE_BATCH, it) }
            }

            override fun onScanFailed(errorCode: Int) {
                Log.w(TAG, "BLE scan failed: $errorCode")
                close(IllegalStateException("BLE scan failed: $errorCode"))
            }
        }

        val filters = listOf(
            ScanFilter.Builder()
                .setServiceUuid(ParcelUuid(serviceUuid))
                .build(),
        )
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
            .setCallbackType(ScanSettings.CALLBACK_TYPE_ALL_MATCHES)
            .also {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                    it.setMatchMode(ScanSettings.MATCH_MODE_AGGRESSIVE)
                }
            }
            .build()

        try {
            scanner.startScan(filters, settings, callback)
        } catch (se: SecurityException) {
            Log.w(TAG, "startScan SecurityException", se)
            close(se)
            return@callbackFlow
        }

        // Initial empty emission so the UI shows "Looking for OHD beacons…"
        // immediately rather than the previous screen's stale list.
        trySend(emptyList())

        awaitClose {
            try {
                scanner.stopScan(callback)
            } catch (e: Throwable) {
                Log.w(TAG, "stopScan failed", e)
            }
        }
    }

    /**
     * Map a [ScanResult] to a [DiscoveredBeacon]. Returns null if the
     * result is missing the OHD service-data payload entirely.
     */
    private fun mapToBeacon(result: ScanResult): DiscoveredBeacon? {
        val record = result.scanRecord ?: return null
        val serviceData = record.getServiceData(ParcelUuid(serviceUuid))
        val beaconId = serviceData?.let { bytes ->
            val take = bytes.take(16).joinToString(separator = "") { byte ->
                "%02x".format(byte.toInt() and 0xFF)
            }
            // Format as 4 colon-separated 4-hex-char groups so it
            // matches the visual style of the mock IDs.
            if (take.length >= 16) take.chunked(4).take(4).joinToString(":") else take
        } ?: result.device.address
        val rssi = result.rssi
        val now = System.currentTimeMillis()
        return DiscoveredBeacon(
            beaconId = beaconId,
            displayLabel = record.deviceName,
            rssiDbm = rssi,
            approximateDistance = approximateDistanceFromRssi(rssi),
            firstSeenAtMs = now,
            lastSeenAtMs = now,
        )
    }
}

private const val SCAN_RESULT_TYPE_BATCH = 2

/** Map RSSI to a paramedic-friendly distance bucket (UI-only heuristic). */
fun approximateDistanceFromRssi(rssi: Int): ApproximateDistance = when {
    rssi >= -55 -> ApproximateDistance.VeryClose
    rssi >= -75 -> ApproximateDistance.Close
    rssi >= -90 -> ApproximateDistance.Nearby
    else -> ApproximateDistance.Far
}

/**
 * Returns true if the runtime permissions for BLE scanning are granted.
 * Pre-Android 12 requires `ACCESS_FINE_LOCATION` (BLE-implies-location);
 * Android 12+ uses `BLUETOOTH_SCAN` with the `neverForLocation` flag (no
 * location prompt).
 */
fun hasBleScanPermission(ctx: Context): Boolean {
    return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        ContextCompat.checkSelfPermission(
            ctx,
            Manifest.permission.BLUETOOTH_SCAN,
        ) == PackageManager.PERMISSION_GRANTED
    } else {
        ContextCompat.checkSelfPermission(
            ctx,
            Manifest.permission.ACCESS_FINE_LOCATION,
        ) == PackageManager.PERMISSION_GRANTED
    }
}

/**
 * The Android-runtime permission name to request, varying by SDK level.
 * Pass to `rememberLauncherForActivityResult` /
 * `ActivityResultContracts.RequestPermission` from Compose.
 */
val bleScanPermissionName: String
    get() = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        Manifest.permission.BLUETOOTH_SCAN
    } else {
        Manifest.permission.ACCESS_FINE_LOCATION
    }
