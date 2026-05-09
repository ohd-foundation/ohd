package com.ohd.connect.data

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import org.json.JSONArray
import org.json.JSONObject

/**
 * Patient-side emergency settings — mirrors connect/web's
 * `EmergencySettingsPage` shape so the two front-ends stay byte-comparable
 * after the eventual `Settings.SetEmergencyConfig` RPC ships.
 *
 * Persistence: `EncryptedSharedPreferences` ("ohd_connect_secure", same
 * vault as the self-session bearer in `Auth.kt`). Single-blob JSON keyed
 * `emergency_config_v1`. v0 stores locally; when the storage core grows
 * `Settings.SetEmergencyConfig` (tracked in `connect/STATUS.md`), the
 * persistence path forks to mirror writes there too.
 *
 * Sections (per `connect/spec/screens-emergency.md`):
 *  1. Feature toggle
 *  2. BLE beacon
 *  3. Approval timeout (s) + default-on-timeout
 *  4. Lock-screen mode
 *  5. History window (h) + per-channel toggles + sensitivity classes
 *  6. Location share
 *  7. Trust roots
 *  8. Bystander proxy
 */
data class EmergencyConfig(
    val featureEnabled: Boolean = false,
    val bleBeacon: Boolean = true,
    val approvalTimeoutSeconds: Int = 30,
    val defaultOnTimeout: DefaultAction = DefaultAction.ALLOW,
    val lockScreenMode: LockScreenMode = LockScreenMode.FULL,
    val historyWindowHours: Int = 24,
    val channels: ChannelToggles = ChannelToggles(),
    val sensitivity: SensitivityToggles = SensitivityToggles(),
    val locationShare: Boolean = false,
    val bystanderProxy: Boolean = true,
    val trustRoots: List<TrustRoot> = listOf(
        TrustRoot(id = "ohd_default", name = "OHD Project (default root)", scope = "global", removable = false),
    ),
) {

    enum class DefaultAction { ALLOW, REFUSE }

    enum class LockScreenMode { FULL, BASIC_ONLY }

    data class ChannelToggles(
        val glucose: Boolean = true,
        val heartRate: Boolean = true,
        val bloodPressure: Boolean = true,
        val spo2: Boolean = true,
        val temperature: Boolean = true,
        val allergies: Boolean = true,
        val medications: Boolean = true,
        val bloodType: Boolean = true,
        val advanceDirectives: Boolean = true,
        val diagnoses: Boolean = true,
    )

    data class SensitivityToggles(
        val general: Boolean = true,
        val mentalHealth: Boolean = false,
        val substanceUse: Boolean = false,
        val sexualHealth: Boolean = false,
        val reproductive: Boolean = false,
    )

    data class TrustRoot(
        val id: String,
        val name: String,
        val scope: String,
        val removable: Boolean,
    )

    fun save(ctx: Context) {
        val obj = JSONObject().apply {
            put("featureEnabled", featureEnabled)
            put("bleBeacon", bleBeacon)
            put("approvalTimeoutSeconds", approvalTimeoutSeconds)
            put("defaultOnTimeout", defaultOnTimeout.name)
            put("lockScreenMode", lockScreenMode.name)
            put("historyWindowHours", historyWindowHours)
            put(
                "channels",
                JSONObject().apply {
                    put("glucose", channels.glucose)
                    put("heartRate", channels.heartRate)
                    put("bloodPressure", channels.bloodPressure)
                    put("spo2", channels.spo2)
                    put("temperature", channels.temperature)
                    put("allergies", channels.allergies)
                    put("medications", channels.medications)
                    put("bloodType", channels.bloodType)
                    put("advanceDirectives", channels.advanceDirectives)
                    put("diagnoses", channels.diagnoses)
                },
            )
            put(
                "sensitivity",
                JSONObject().apply {
                    put("general", sensitivity.general)
                    put("mentalHealth", sensitivity.mentalHealth)
                    put("substanceUse", sensitivity.substanceUse)
                    put("sexualHealth", sensitivity.sexualHealth)
                    put("reproductive", sensitivity.reproductive)
                },
            )
            put("locationShare", locationShare)
            put("bystanderProxy", bystanderProxy)
            put(
                "trustRoots",
                JSONArray().apply {
                    trustRoots.forEach {
                        put(
                            JSONObject().apply {
                                put("id", it.id)
                                put("name", it.name)
                                put("scope", it.scope)
                                put("removable", it.removable)
                            },
                        )
                    }
                },
            )
        }
        prefs(ctx).edit().putString(KEY, obj.toString()).apply()
    }

    companion object {

        private const val KEY = "emergency_config_v1"
        private const val PREF_NAME = "ohd_connect_secure"

        fun load(ctx: Context): EmergencyConfig {
            val raw = prefs(ctx).getString(KEY, null) ?: return EmergencyConfig()
            return runCatching {
                val obj = JSONObject(raw)
                EmergencyConfig(
                    featureEnabled = obj.optBoolean("featureEnabled", false),
                    bleBeacon = obj.optBoolean("bleBeacon", true),
                    approvalTimeoutSeconds = obj.optInt("approvalTimeoutSeconds", 30),
                    defaultOnTimeout = runCatching {
                        DefaultAction.valueOf(obj.optString("defaultOnTimeout", "ALLOW"))
                    }.getOrDefault(DefaultAction.ALLOW),
                    lockScreenMode = runCatching {
                        LockScreenMode.valueOf(obj.optString("lockScreenMode", "FULL"))
                    }.getOrDefault(LockScreenMode.FULL),
                    historyWindowHours = obj.optInt("historyWindowHours", 24),
                    channels = obj.optJSONObject("channels")?.let { c ->
                        ChannelToggles(
                            glucose = c.optBoolean("glucose", true),
                            heartRate = c.optBoolean("heartRate", true),
                            bloodPressure = c.optBoolean("bloodPressure", true),
                            spo2 = c.optBoolean("spo2", true),
                            temperature = c.optBoolean("temperature", true),
                            allergies = c.optBoolean("allergies", true),
                            medications = c.optBoolean("medications", true),
                            bloodType = c.optBoolean("bloodType", true),
                            advanceDirectives = c.optBoolean("advanceDirectives", true),
                            diagnoses = c.optBoolean("diagnoses", true),
                        )
                    } ?: ChannelToggles(),
                    sensitivity = obj.optJSONObject("sensitivity")?.let { s ->
                        SensitivityToggles(
                            general = s.optBoolean("general", true),
                            mentalHealth = s.optBoolean("mentalHealth", false),
                            substanceUse = s.optBoolean("substanceUse", false),
                            sexualHealth = s.optBoolean("sexualHealth", false),
                            reproductive = s.optBoolean("reproductive", false),
                        )
                    } ?: SensitivityToggles(),
                    locationShare = obj.optBoolean("locationShare", false),
                    bystanderProxy = obj.optBoolean("bystanderProxy", true),
                    trustRoots = obj.optJSONArray("trustRoots")?.let { arr ->
                        (0 until arr.length()).map { i ->
                            val r = arr.getJSONObject(i)
                            TrustRoot(
                                id = r.optString("id"),
                                name = r.optString("name"),
                                scope = r.optString("scope"),
                                removable = r.optBoolean("removable", true),
                            )
                        }
                    } ?: EmergencyConfig().trustRoots,
                )
            }.getOrElse { EmergencyConfig() }
        }

        private fun prefs(ctx: Context) = runCatching {
            val masterKey = MasterKey.Builder(ctx)
                .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
                .build()
            EncryptedSharedPreferences.create(
                ctx,
                PREF_NAME,
                masterKey,
                EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
                EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
            )
        }.getOrElse {
            // Fall back to plain prefs if Keystore is unavailable (mirrors
            // the recovery path in Auth.kt).
            ctx.getSharedPreferences("ohd_connect_state", Context.MODE_PRIVATE)
        }
    }
}
