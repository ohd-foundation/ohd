# Research: Android Health Connect Integration

> Target platform for Phase 1 data collection. How to actually read data out of it into OHD.

## What Health Connect is

Health Connect is a health data platform for Android app developers that provides a single, consolidated interface for access to users' health and fitness data, with consistent functional behavior across all devices. It supports more than 50 data types (activity, sleep, nutrition, body measurements, vitals) and is the successor to the deprecated Google Fit APIs — Google is transitioning away from Google Fit APIs starting in 2026.

On Android 14+ (API 34), Health Connect ships as part of the Android framework. On Android 13 and earlier, it's a separate app from the Play Store. Our minimum target is Android 10 (API 29), which requires Health Connect to be installed from the Play Store.

**Why Health Connect is the right integration point:** Samsung Health, Google Fit, Fitbit, Garmin Connect, Libre Link, Xiaomi Mi Fit, and many more wearable/medical apps all write to Health Connect. Samsung Health syncs with Health Connect bidirectionally — when there's new data in Health Connect, Samsung Health retrieves and saves it, and vice versa. Rather than integrating with each app individually, we integrate once with Health Connect and benefit from everyone else's integrations.

## Data types we care about (Phase 1)

From the Health Connect catalog, the types most relevant to OHD MVP:

### Vitals
- `HeartRateRecord` — series of samples with BPM values and timestamps
- `RestingHeartRateRecord` — single measurement
- `BloodGlucoseRecord` — glucose readings; includes `mealType` and `specimenSource`
- `BloodPressureRecord` — systolic + diastolic
- `BodyTemperatureRecord` — with measurement location
- `OxygenSaturationRecord` — SpO2 percentage
- `RespiratoryRateRecord`
- `HeartRateVariabilityRmssdRecord` — HRV

### Body measurements
- `WeightRecord`
- `BodyFatRecord`
- `BodyWaterMassRecord`
- `BoneMassRecord`
- `LeanBodyMassRecord`
- `HeightRecord`

### Activity
- `StepsRecord`
- `DistanceRecord`
- `ActiveCaloriesBurnedRecord`
- `TotalCaloriesBurnedRecord`
- `ExerciseSessionRecord` — a session with optional embedded `HeartRateRecord` samples and `StepsRecord`

### Sleep
- `SleepSessionRecord` — with optional stage segments

### Nutrition
- `NutritionRecord` — one meal/intake event with macro + micro breakdowns
- `HydrationRecord` — fluid volume

### Cycle tracking
- `MenstruationPeriodRecord`, `MenstruationFlowRecord`, `CervicalMucusRecord`, `OvulationTestRecord`, `IntermenstrualBleedingRecord`, `SexualActivityRecord`

### Others likely relevant later
- `Vo2MaxRecord`, `PowerRecord`, `SpeedRecord`, `ElevationGainedRecord`, `FloorsClimbedRecord`, `WheelchairPushesRecord`

## Record shapes

Health Connect distinguishes three kinds of records:

1. **Instantaneous records** — one measurement at a point in time (heart rate reading, weight, glucose). Fields: `time`, `zoneOffset`, value fields.
2. **Interval records** — a measurement over a period (steps in a 5-minute window, active calories during an exercise). Fields: `startTime`, `startZoneOffset`, `endTime`, `endZoneOffset`, value fields.
3. **Series records** — a series of samples over a period (heart rate samples during a run, per-minute steps). Embedded list of `{time, value}` entries.

This maps directly onto OHD's event model:

- Instantaneous → OHD event with `timestamp`, no `duration_seconds`.
- Interval → OHD event with `timestamp = startTime`, `duration_seconds = endTime - startTime`.
- Series → OHD series event with `data.samples = [...]` or, alternatively, expanded into one OHD event per sample (user preference / Connector config).

## Permissions

Permissions in Health Connect are per data type and per direction (read/write).

### Manifest declarations

```xml
<!-- AndroidManifest.xml -->
<uses-permission android:name="android.permission.health.READ_HEART_RATE"/>
<uses-permission android:name="android.permission.health.READ_BLOOD_GLUCOSE"/>
<uses-permission android:name="android.permission.health.READ_BLOOD_PRESSURE"/>
<uses-permission android:name="android.permission.health.READ_BODY_TEMPERATURE"/>
<uses-permission android:name="android.permission.health.READ_WEIGHT"/>
<uses-permission android:name="android.permission.health.READ_STEPS"/>
<uses-permission android:name="android.permission.health.READ_DISTANCE"/>
<uses-permission android:name="android.permission.health.READ_SLEEP"/>
<uses-permission android:name="android.permission.health.READ_EXERCISE"/>
<uses-permission android:name="android.permission.health.READ_NUTRITION"/>
<uses-permission android:name="android.permission.health.READ_HYDRATION"/>
<uses-permission android:name="android.permission.health.READ_OXYGEN_SATURATION"/>
<uses-permission android:name="android.permission.health.READ_HEART_RATE_VARIABILITY"/>
<uses-permission android:name="android.permission.health.READ_BODY_FAT"/>
<uses-permission android:name="android.permission.health.READ_HEIGHT"/>
<uses-permission android:name="android.permission.health.READ_RESPIRATORY_RATE"/>
<uses-permission android:name="android.permission.health.READ_RESTING_HEART_RATE"/>
<uses-permission android:name="android.permission.health.READ_ACTIVE_CALORIES_BURNED"/>
<uses-permission android:name="android.permission.health.READ_TOTAL_CALORIES_BURNED"/>
<uses-permission android:name="android.permission.health.READ_VO2_MAX"/>

<!-- Required if we want to read data older than 30 days -->
<uses-permission android:name="android.permission.health.READ_HEALTH_DATA_HISTORY"/>

<!-- Required if we want to sync while the app is in the background -->
<uses-permission android:name="android.permission.health.READ_HEALTH_DATA_IN_BACKGROUND"/>
```

### History permission

By default, all applications can read data from Health Connect for up to 30 days prior to when any permission was first granted. To extend read permissions beyond this default restriction, request PERMISSION_READ_HEALTH_DATA_HISTORY. Without this permission, an attempt to read records older than 30 days results in an error.

We definitely need this — users will want to backfill existing historical data into their OHD instance on first setup.

### Requesting permissions at runtime

```kotlin
val permissions = setOf(
    HealthPermission.getReadPermission(HeartRateRecord::class),
    HealthPermission.getReadPermission(BloodGlucoseRecord::class),
    HealthPermission.getReadPermission(WeightRecord::class),
    // ... all the types we want
    "android.permission.health.READ_HEALTH_DATA_HISTORY",
    "android.permission.health.READ_HEALTH_DATA_IN_BACKGROUND"
)

val requestPermissionsLauncher = registerForActivityResult(
    PermissionController.createRequestPermissionResultContract()
) { grantedPermissions ->
    if (grantedPermissions.containsAll(permissions)) {
        // Start sync
    } else {
        // Show "you need to grant these to use OHD sync" UI
    }
}

// Later:
lifecycleScope.launch {
    val granted = healthConnectClient.permissionController.getGrantedPermissions()
    if (!granted.containsAll(permissions)) {
        requestPermissionsLauncher.launch(permissions)
    }
}
```

### Rationale activity (required)

We must declare an Activity that handles `ACTION_SHOW_PERMISSIONS_RATIONALE` — this is shown when the user taps the privacy policy link on the Health Connect permissions screen.

```xml
<activity android:name=".PrivacyPolicyActivity" android:exported="true">
    <intent-filter>
        <action android:name="androidx.health.ACTION_SHOW_PERMISSIONS_RATIONALE"/>
    </intent-filter>
</activity>
```

## Reading data

### Basic read

```kotlin
val client = HealthConnectClient.getOrCreate(context)

val response = client.readRecords(
    ReadRecordsRequest(
        recordType = BloodGlucoseRecord::class,
        timeRangeFilter = TimeRangeFilter.between(startTime, endTime),
        pageSize = 1000
    )
)

for (record in response.records) {
    // record.time, record.level, record.mealType, record.specimenSource, ...
}

// Pagination via response.pageToken for the next page
```

### Filtering by source app

If we want to know which app produced the data (for `metadata.source`), use `dataOriginFilter`:

```kotlin
val response = client.readRecords(
    ReadRecordsRequest(
        recordType = HeartRateRecord::class,
        timeRangeFilter = TimeRangeFilter.between(startTime, endTime),
        dataOriginFilter = setOf(DataOrigin("com.sec.android.app.shealth"))
    )
)
```

Every record has a `Metadata` object with `dataOrigin.packageName`. We'll use this to fill OHD's `metadata.source` field.

### Incremental sync with change tokens

This is critical for a periodic sync worker — don't re-read everything every time.

```kotlin
// On first sync
val changesTokenRequest = ChangesTokenRequest(setOf(BloodGlucoseRecord::class))
var token = client.getChangesToken(changesTokenRequest)

// Persist token. On next sync:
val changes = client.getChanges(token)
for (change in changes.changes) {
    when (change) {
        is UpsertionChange -> handleUpsert(change.record)
        is DeletionChange -> handleDeletion(change.recordId)
    }
}
// Update saved token
if (!changes.changesTokenExpired) {
    token = changes.nextChangesToken
} else {
    // Token expired (happens after ~30 days of no sync); do a full read.
}
```

## Writing data back (Phase 2+)

We can also write into Health Connect. Interesting case: when a user manually logs food in OHDC, we can optionally mirror it into Health Connect as a `NutritionRecord`, so other apps see it. Requires corresponding `WRITE_*` permissions.

## Best practices from the docs

- For data types that use a series of samples, such as HeartRateRecord, structure records correctly: instead of creating a single, day-long record that is constantly updated, create multiple smaller records, each representing a specific time interval. For example, for heart rate data, create a new HeartRateRecord for each minute.
- On every sync, only write new data and updated data modified since the last sync. Chunk requests to at most 1000 records per write request. Restrict tasks to run only when the device is idle and not low on battery.
- Use WorkManager for periodic background sync with a minimum interval of 15 minutes (Android's lower bound for periodic work).

## Sync architecture for OHDC Android

```
┌─────────────────────────────────────────────┐
│  OHDC Android App                            │
│                                              │
│  ┌──────────────────┐   ┌─────────────────┐ │
│  │  Sync UI         │   │  Manual Log UI  │ │
│  └────────┬─────────┘   └────────┬────────┘ │
│           │                      │          │
│  ┌────────▼──────────────────────▼────────┐ │
│  │  Repository                            │ │
│  │  - talks to Health Connect             │ │
│  │  - talks to OHD API                    │ │
│  │  - maintains local queue (Room/SQLite) │ │
│  └────┬─────────────────────────────┬────┘ │
│       │                             │       │
│  ┌────▼─────────────┐       ┌───────▼─────┐│
│  │  SyncWorker      │       │  Local queue│ │
│  │  (WorkManager,   │       │  (SQLite)   │ │
│  │   every 30 min)  │       └─────────────┘ │
│  └──────────────────┘                       │
└─────────────────────────────────────────────┘
         │                    │
         ▼                    ▼
  Health Connect         OHD API
```

## Dependencies

```kotlin
// app/build.gradle.kts
dependencies {
    implementation("androidx.health.connect:connect-client:1.1.0-rc03")

    // WorkManager for periodic sync
    implementation("androidx.work:work-runtime-ktx:2.10.0")

    // HTTP client for talking to OHD API
    implementation("com.squareup.okhttp3:okhttp:5.0.0")
    implementation("com.squareup.retrofit2:retrofit:2.11.0")
    implementation("com.squareup.retrofit2:converter-moshi:2.11.0")

    // Local queue database
    implementation("androidx.room:room-runtime:2.6.1")
    implementation("androidx.room:room-ktx:2.6.1")
    ksp("androidx.room:room-compiler:2.6.1")

    // Compose for UI
    implementation(platform("androidx.compose:compose-bom:2024.12.01"))
    implementation("androidx.compose.material3:material3")
    // ...
}
```

## Play Store publication considerations (future)

The Play Store requires a **Health Apps declaration** in Play Console before an app using Health Connect permissions can be published. This is a formal review: we have to declare what data types we access, why, and have a privacy policy.

For personal/developer use (sideloaded, or Internal Testing track), this isn't required. We'll worry about publication when we have something to publish.

## Open questions / research gaps

- **Exercise session embedded data.** `ExerciseSessionRecord` can embed heart rate series, steps, etc. Do we import the session as one OHD event with nested series, or flatten into multiple events? Probably the former for fidelity, with a Cord feature to flatten for analysis.
- **Nutrition record mapping.** Health Connect's `NutritionRecord` has a fixed schema. Our OpenFoodFacts-sourced food events have richer data. We probably write only Health Connect-originated nutrition to OHD as `meal` events, and let manual food logging bypass Health Connect's schema.
- **Deduplication.** If the Xiaomi app writes a heart rate reading to Health Connect, Samsung Health reads it and re-writes it — we may see duplicates. Need to check how Health Connect handles this or implement app-level dedup via `(source_id, timestamp)`.
- **Rate of writes to OHD.** A user with a CGM generating 4,320 readings/day and a smartwatch generating 28,800 heart rate readings/day adds up to ~33K events/day. Batched uploads with idempotency keys are essential.
