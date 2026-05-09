package com.ohd.emergency.data

import android.content.Context
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import android.util.Log
import org.json.JSONObject

/**
 * Persistent store for [CaseVault.QueuedWrite].
 *
 * Per `STATUS.md` "What's mocked → uniffi case-vault cache":
 *
 *     The patient-data slice stays RAM-only; only the queue persists,
 *     and only its un-submitted rows.
 *
 * Goal: a tablet reboot mid-shift (driver crashed, low battery, app
 * killed) should not drop queued intervention writes. On app restart we
 * load any unflushed rows back into [CaseVault.queuedWrites] so the
 * background flush worker can drain them.
 *
 * # Why hand-rolled SQLite, not Room
 *
 * Room would pull KSP + a code-generation pass into the build. The
 * schema is one table; SQLiteOpenHelper is ~80 lines and avoids the
 * compile-time dependency. Same call shape from the repository layer.
 *
 * # Why not the uniffi case-vault cache
 *
 * The uniffi cache (Stage 1 + Stage 2 of `BUILD.md`) is the production
 * target eventually; it would store this queue alongside the cached
 * patient slice. v0 doesn't ship those `.so` files, so the queue here
 * is a separate plain-SQLite table that lands a sibling drawer in the
 * cache file when the uniffi path comes online.
 *
 * # Schema
 *
 * ```sql
 * CREATE TABLE queued_writes (
 *   local_ulid    TEXT PRIMARY KEY NOT NULL,
 *   case_ulid     TEXT NOT NULL,
 *   occurred_ms   INTEGER NOT NULL,
 *   recorded_ms   INTEGER NOT NULL,
 *   kind          TEXT NOT NULL,             -- Vital / Drug / Observation / Note
 *   summary       TEXT NOT NULL,
 *   payload_json  TEXT NOT NULL              -- serialized InterventionPayload
 * );
 * CREATE INDEX idx_queued_case ON queued_writes(case_ulid, recorded_ms);
 * ```
 *
 * The `payload_json` column carries the [CaseVault.InterventionPayload]
 * variant + its fields. Schema changes bump `DATABASE_VERSION` and add
 * a migration in [onUpgrade].
 *
 * # Backup exclusion
 *
 * The DB file `case_vault.db` is in the standard `databases/` dir; the
 * data-extraction-rules already exclude `case_vault.db*` from auto-backup
 * (per `data_extraction_rules.xml`). Queued writes belong to the active
 * paramedic crew on this device and never leave it.
 */
class QueuedWriteStore(ctx: Context) : SQLiteOpenHelper(
    ctx.applicationContext,
    DATABASE_NAME,
    null,
    DATABASE_VERSION,
) {
    companion object {
        private const val TAG = "OhdEmergency.QueuedWriteStore"
        const val DATABASE_NAME = "case_vault.db"
        const val DATABASE_VERSION = 1

        const val TABLE = "queued_writes"
        const val COL_LOCAL_ULID = "local_ulid"
        const val COL_CASE_ULID = "case_ulid"
        const val COL_OCCURRED_MS = "occurred_ms"
        const val COL_RECORDED_MS = "recorded_ms"
        const val COL_KIND = "kind"
        const val COL_SUMMARY = "summary"
        const val COL_PAYLOAD_JSON = "payload_json"
    }

    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL("""
            CREATE TABLE IF NOT EXISTS $TABLE (
              $COL_LOCAL_ULID TEXT PRIMARY KEY NOT NULL,
              $COL_CASE_ULID TEXT NOT NULL,
              $COL_OCCURRED_MS INTEGER NOT NULL,
              $COL_RECORDED_MS INTEGER NOT NULL,
              $COL_KIND TEXT NOT NULL,
              $COL_SUMMARY TEXT NOT NULL,
              $COL_PAYLOAD_JSON TEXT NOT NULL
            )
        """.trimIndent())
        db.execSQL("CREATE INDEX IF NOT EXISTS idx_queued_case ON $TABLE($COL_CASE_ULID, $COL_RECORDED_MS)")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) {
        // v0: no schema migrations yet. When the schema evolves, add
        // ALTER TABLE here keyed on `oldVersion`.
        if (oldVersion < newVersion) {
            Log.i(TAG, "onUpgrade $oldVersion -> $newVersion (no migrations defined)")
        }
    }

    fun insert(write: CaseVault.QueuedWrite) {
        val db = writableDatabase
        val sql = """
            INSERT OR REPLACE INTO $TABLE
            ($COL_LOCAL_ULID, $COL_CASE_ULID, $COL_OCCURRED_MS, $COL_RECORDED_MS,
             $COL_KIND, $COL_SUMMARY, $COL_PAYLOAD_JSON)
            VALUES (?, ?, ?, ?, ?, ?, ?)
        """.trimIndent()
        db.execSQL(sql, arrayOf(
            write.localUlid,
            write.caseUlid,
            write.occurredAtMs,
            write.recordedAtMs,
            write.kind.name,
            write.summary,
            payloadToJson(write.payload),
        ))
    }

    fun deleteByLocalUlid(localUlid: String) {
        writableDatabase.delete(TABLE, "$COL_LOCAL_ULID = ?", arrayOf(localUlid))
    }

    fun deleteAllForCase(caseUlid: String) {
        writableDatabase.delete(TABLE, "$COL_CASE_ULID = ?", arrayOf(caseUlid))
    }

    fun deleteAll() {
        writableDatabase.delete(TABLE, null, null)
    }

    /** Load every queued write, oldest first (so flush order matches recording order). */
    fun loadAll(): List<CaseVault.QueuedWrite> {
        val out = mutableListOf<CaseVault.QueuedWrite>()
        readableDatabase.query(
            TABLE,
            null,
            null, null, null, null,
            "$COL_RECORDED_MS ASC",
        ).use { c ->
            val iLocal = c.getColumnIndexOrThrow(COL_LOCAL_ULID)
            val iCase = c.getColumnIndexOrThrow(COL_CASE_ULID)
            val iOccurred = c.getColumnIndexOrThrow(COL_OCCURRED_MS)
            val iRecorded = c.getColumnIndexOrThrow(COL_RECORDED_MS)
            val iKind = c.getColumnIndexOrThrow(COL_KIND)
            val iSummary = c.getColumnIndexOrThrow(COL_SUMMARY)
            val iPayload = c.getColumnIndexOrThrow(COL_PAYLOAD_JSON)
            while (c.moveToNext()) {
                runCatching {
                    out.add(CaseVault.QueuedWrite(
                        localUlid = c.getString(iLocal),
                        caseUlid = c.getString(iCase),
                        occurredAtMs = c.getLong(iOccurred),
                        recordedAtMs = c.getLong(iRecorded),
                        kind = CaseVault.InterventionKind.valueOf(c.getString(iKind)),
                        summary = c.getString(iSummary),
                        payload = payloadFromJson(c.getString(iKind), c.getString(iPayload)),
                    ))
                }.onFailure { Log.w(TAG, "loadAll skip: ${it.message}") }
            }
        }
        return out
    }

    // ----- Payload (de)serialization ---------------------------------------

    private fun payloadToJson(p: CaseVault.InterventionPayload): String = when (p) {
        is CaseVault.InterventionPayload.Vital -> JSONObject().apply {
            put("kind", "Vital")
            put("channel", p.channel)
            put("value", p.value)
            put("unit", p.unit)
        }.toString()
        is CaseVault.InterventionPayload.BloodPressure -> JSONObject().apply {
            put("kind", "BloodPressure")
            put("systolic", p.systolic)
            put("diastolic", p.diastolic)
        }.toString()
        is CaseVault.InterventionPayload.Drug -> JSONObject().apply {
            put("kind", "Drug")
            put("name", p.name)
            put("doseValue", p.doseValue)
            put("doseUnit", p.doseUnit)
            put("route", p.route)
        }.toString()
        is CaseVault.InterventionPayload.Observation -> JSONObject().apply {
            put("kind", "Observation")
            put("freeText", p.freeText)
            if (p.gcs != null) put("gcs", p.gcs)
            if (p.skinColor != null) put("skinColor", p.skinColor)
        }.toString()
        is CaseVault.InterventionPayload.Note -> JSONObject().apply {
            put("kind", "Note")
            put("text", p.text)
        }.toString()
    }

    private fun payloadFromJson(kindEnum: String, json: String): CaseVault.InterventionPayload {
        val j = JSONObject(json)
        return when (j.optString("kind", kindEnum)) {
            "Vital" -> CaseVault.InterventionPayload.Vital(
                channel = j.optString("channel"),
                value = j.optDouble("value"),
                unit = j.optString("unit"),
            )
            "BloodPressure" -> CaseVault.InterventionPayload.BloodPressure(
                systolic = j.optInt("systolic"),
                diastolic = j.optInt("diastolic"),
            )
            "Drug" -> CaseVault.InterventionPayload.Drug(
                name = j.optString("name"),
                doseValue = j.optDouble("doseValue"),
                doseUnit = j.optString("doseUnit"),
                route = j.optString("route"),
            )
            "Observation" -> CaseVault.InterventionPayload.Observation(
                freeText = j.optString("freeText"),
                gcs = j.optInt("gcs", Int.MIN_VALUE).takeIf { it != Int.MIN_VALUE },
                skinColor = j.optString("skinColor").takeIf { it.isNotEmpty() },
            )
            "Note" -> CaseVault.InterventionPayload.Note(text = j.optString("text"))
            else -> CaseVault.InterventionPayload.Note(text = "[unparseable payload]")
        }
    }
}
