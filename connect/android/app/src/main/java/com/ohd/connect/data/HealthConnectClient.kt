package com.ohd.connect.data

import android.content.Context
import androidx.health.connect.client.HealthConnectClient
import androidx.health.connect.client.PermissionController
import androidx.health.connect.client.permission.HealthPermission
import androidx.health.connect.client.records.ActiveCaloriesBurnedRecord
import androidx.health.connect.client.records.BasalBodyTemperatureRecord
import androidx.health.connect.client.records.BasalMetabolicRateRecord
import androidx.health.connect.client.records.BloodGlucoseRecord
import androidx.health.connect.client.records.BloodPressureRecord
import androidx.health.connect.client.records.BodyFatRecord
import androidx.health.connect.client.records.BodyTemperatureRecord
import androidx.health.connect.client.records.BodyWaterMassRecord
import androidx.health.connect.client.records.BoneMassRecord
import androidx.health.connect.client.records.CyclingPedalingCadenceRecord
import androidx.health.connect.client.records.DistanceRecord
import androidx.health.connect.client.records.ElevationGainedRecord
import androidx.health.connect.client.records.ExerciseSessionRecord
import androidx.health.connect.client.records.FloorsClimbedRecord
import androidx.health.connect.client.records.HeartRateRecord
import androidx.health.connect.client.records.HeartRateVariabilityRmssdRecord
import androidx.health.connect.client.records.HeightRecord
import androidx.health.connect.client.records.HydrationRecord
import androidx.health.connect.client.records.LeanBodyMassRecord
import androidx.health.connect.client.records.NutritionRecord
import androidx.health.connect.client.records.OxygenSaturationRecord
import androidx.health.connect.client.records.PowerRecord
import androidx.health.connect.client.records.RespiratoryRateRecord
import androidx.health.connect.client.records.RestingHeartRateRecord
import androidx.health.connect.client.records.SleepSessionRecord
import androidx.health.connect.client.records.SpeedRecord
import androidx.health.connect.client.records.StepsCadenceRecord
import androidx.health.connect.client.records.StepsRecord
import androidx.health.connect.client.records.TotalCaloriesBurnedRecord
import androidx.health.connect.client.records.Vo2MaxRecord
import androidx.health.connect.client.records.WeightRecord
import androidx.health.connect.client.records.WheelchairPushesRecord

/**
 * Thin wrapper around [androidx.health.connect.client.HealthConnectClient].
 *
 * The Health Connect SDK has three distinct surfaces:
 *
 *   1. Provider availability  — `HealthConnectClient.getSdkStatus(ctx)`
 *      returns one of `SDK_AVAILABLE`, `SDK_UNAVAILABLE_PROVIDER_UPDATE_REQUIRED`,
 *      or `SDK_UNAVAILABLE`. We map those to [Availability] for the
 *      Settings screen.
 *   2. Permission negotiation — the Health Connect app owns the
 *      permission UI; the SDK exposes a `PermissionController` and an
 *      [androidx.activity.result.contract.ActivityResultContract] that
 *      hands a `Set<String>` of "android.permission.health.READ_*" strings
 *      back and forth.
 *   3. Read APIs              — `HealthConnectClient.readRecords(...)`
 *      with a typed [androidx.health.connect.client.request.ReadRecordsRequest].
 *      Lives in [HealthConnectSync].
 *
 * Keep this object surface-only: no `suspend` IO that goes beyond
 * `getGrantedPermissions`. The actual record-reading lives in
 * [HealthConnectSync] so the wrapper stays trivially testable.
 */
object OhdHealthConnect {

    /**
     * Availability of the Health Connect provider on the device.
     *
     * The [Installed] case still requires that the user grant the OHD app
     * read permissions inside the Health Connect app — see
     * [grantedPermissions]. The Settings screen treats those as orthogonal
     * states so the user can act on each independently.
     */
    enum class Availability { Installed, NeedsUpdate, NotInstalled }

    /**
     * The exact `android.permission.health.READ_*` strings we declare in
     * `AndroidManifest.xml`. They MUST match the manifest one-for-one or
     * the Health Connect permission dialog silently drops them.
     *
     * Each entry uses `HealthPermission.getReadPermission(...)` rather
     * than a hard-coded literal so a future Health Connect SDK rename
     * propagates here automatically.
     */
    val PermissionsRead: Set<String> = setOf(
        // Vitals
        HealthPermission.getReadPermission(HeartRateRecord::class),
        HealthPermission.getReadPermission(RestingHeartRateRecord::class),
        HealthPermission.getReadPermission(HeartRateVariabilityRmssdRecord::class),
        HealthPermission.getReadPermission(BloodPressureRecord::class),
        HealthPermission.getReadPermission(BloodGlucoseRecord::class),
        HealthPermission.getReadPermission(OxygenSaturationRecord::class),
        HealthPermission.getReadPermission(RespiratoryRateRecord::class),
        HealthPermission.getReadPermission(BodyTemperatureRecord::class),
        HealthPermission.getReadPermission(BasalBodyTemperatureRecord::class),
        // Body
        HealthPermission.getReadPermission(WeightRecord::class),
        HealthPermission.getReadPermission(HeightRecord::class),
        HealthPermission.getReadPermission(BodyFatRecord::class),
        HealthPermission.getReadPermission(BodyWaterMassRecord::class),
        HealthPermission.getReadPermission(BoneMassRecord::class),
        HealthPermission.getReadPermission(LeanBodyMassRecord::class),
        // Activity
        HealthPermission.getReadPermission(StepsRecord::class),
        HealthPermission.getReadPermission(StepsCadenceRecord::class),
        HealthPermission.getReadPermission(DistanceRecord::class),
        HealthPermission.getReadPermission(ElevationGainedRecord::class),
        HealthPermission.getReadPermission(FloorsClimbedRecord::class),
        HealthPermission.getReadPermission(ExerciseSessionRecord::class),
        HealthPermission.getReadPermission(ActiveCaloriesBurnedRecord::class),
        HealthPermission.getReadPermission(TotalCaloriesBurnedRecord::class),
        HealthPermission.getReadPermission(BasalMetabolicRateRecord::class),
        HealthPermission.getReadPermission(Vo2MaxRecord::class),
        HealthPermission.getReadPermission(PowerRecord::class),
        HealthPermission.getReadPermission(SpeedRecord::class),
        HealthPermission.getReadPermission(CyclingPedalingCadenceRecord::class),
        HealthPermission.getReadPermission(WheelchairPushesRecord::class),
        // Sleep
        HealthPermission.getReadPermission(SleepSessionRecord::class),
        // Nutrition
        HealthPermission.getReadPermission(NutritionRecord::class),
        HealthPermission.getReadPermission(HydrationRecord::class),
        // Special access — background sync + history beyond the 30-day cap.
        // `PERMISSION_READ_HEALTH_DATA_HISTORY` only became a public Kotlin
        // constant in HC 1.1.0-rc01+. Alpha07 still recognises the literal
        // permission string on the device side, so we declare it explicitly.
        HealthPermission.PERMISSION_READ_HEALTH_DATA_IN_BACKGROUND,
        "android.permission.health.READ_HEALTH_DATA_HISTORY",
    )

    /**
     * Snapshot of provider status. Cheap — no suspend needed; safe to call
     * during recomposition. The underlying `getSdkStatus` is a synchronous
     * `PackageManager` lookup.
     */
    fun availability(ctx: Context): Availability {
        return when (HealthConnectClient.getSdkStatus(ctx, HEALTHDATA_PACKAGE)) {
            HealthConnectClient.SDK_AVAILABLE -> Availability.Installed
            HealthConnectClient.SDK_UNAVAILABLE_PROVIDER_UPDATE_REQUIRED -> Availability.NeedsUpdate
            else -> Availability.NotInstalled
        }
    }

    /**
     * Returns the Health Connect client, or `null` if the provider isn't
     * installed / needs update. Callers should always null-check; the
     * Settings screen short-circuits to the install-link state on null.
     */
    fun client(ctx: Context): HealthConnectClient? {
        return if (availability(ctx) == Availability.Installed) {
            HealthConnectClient.getOrCreate(ctx)
        } else {
            null
        }
    }

    /**
     * Currently-granted permissions, as the same string set we declared in
     * the manifest. The Health Connect provider returns the literal
     * permission strings — no SDK abstraction in between — so we can
     * intersect this with [PermissionsRead] to compute "X of Y granted".
     *
     * Returns the empty set when the provider isn't installed.
     */
    suspend fun grantedPermissions(ctx: Context): Set<String> {
        val c = client(ctx) ?: return emptySet()
        return c.permissionController.getGrantedPermissions()
    }

    /**
     * Convenience wrapper around [PermissionController.createRequestPermissionResultContract].
     *
     * The contract hands a `Set<String>` to the Health Connect provider and
     * receives the granted subset back as a `Set<String>`. The contract is
     * stateless — fresh instance per call is fine.
     */
    fun requestPermissionContract() =
        PermissionController.createRequestPermissionResultContract()

    /** Package name of the Health Connect provider on Play-store devices. */
    const val HEALTHDATA_PACKAGE: String = "com.google.android.apps.healthdata"

    /**
     * Play Store deep link for the Health Connect provider — used by the
     * "Install Health Connect" button when [availability] is
     * [Availability.NotInstalled]. The `market://` scheme bounces through
     * the Play Store app on devices that have it; falls back to the web
     * URL is up to the caller (we render two URIs and let the system
     * resolver choose the one it can handle).
     */
    const val PLAY_STORE_URI: String = "market://details?id=$HEALTHDATA_PACKAGE"
    const val PLAY_STORE_WEB_URL: String =
        "https://play.google.com/store/apps/details?id=$HEALTHDATA_PACKAGE"
}
