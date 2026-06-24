package com.ohd.connect.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdInput
import com.ohd.connect.ui.components.OhdMedLogItem
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TakenState
import com.ohd.connect.ui.components.TopBarAction
import com.ohd.connect.ui.screens._shared.AddOnHandDialog
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * Medications — the active-regimen surface.
 *
 * Regimens (the medications the user is currently on) are real persisted
 * state now, not stub data: read from `list_active_regimens` and written
 * through `start_medication_regimen` / `discontinue_medication_regimen` /
 * `log_medication` — the same MCP tools Claude/Gemini call, via
 * [StorageRepository.executeToolJson]. A regimen started in chat shows up
 * here and vice-versa.
 *
 * Dose logging follows [[project-no-judgment-logging]]: the dialog
 * records the ACTUAL dose taken (prefilled from the regimen but freely
 * editable) and gives **Skip** equal billing with **Log dose** — a
 * missed dose is first-class data, not a failure. No schedule-derived
 * "missed" shaming: frequency is free text we don't police.
 */
@Composable
fun MedicationScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onOpenLibrary: () -> Unit,
    onLogMedication: (String) -> Unit = {},
    onToast: (String) -> Unit = {},
) {
    val scope = rememberCoroutineScope()
    var refreshTick by remember { mutableStateOf(0) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }
    var regimens by remember { mutableStateOf<List<Regimen>>(emptyList()) }
    // regimen_id → most-recent medication.taken timestamp.
    var lastDoseAt by remember { mutableStateOf<Map<String, Long>>(emptyMap()) }

    var actionTarget by remember { mutableStateOf<Regimen?>(null) }
    var addOpen by remember { mutableStateOf(false) }

    suspend fun reload() {
        val regsRes = StorageRepository.executeToolJson("list_active_regimens", "{}")
        val dosesRes = StorageRepository.executeToolJson(
            "query_events",
            JSONObject().put("event_type", "medication.taken")
                .put("visibility", "all").put("limit", 500).toString(),
        )
        withContext(Dispatchers.Main) {
            regsRes.fold(
                onSuccess = { raw ->
                    runCatching {
                        val arr = JSONObject(raw).optJSONArray("regimens")
                        regimens = (0 until (arr?.length() ?: 0)).mapNotNull { i ->
                            val o = arr!!.optJSONObject(i) ?: return@mapNotNull null
                            val id = o.optString("regimen_id", "").ifEmpty { return@mapNotNull null }
                            Regimen(
                                regimenId = id,
                                name = o.optString("name", "Medication"),
                                doseValue = if (o.has("dose_value")) o.optDouble("dose_value") else null,
                                doseUnit = o.optString("dose_unit", "").ifEmpty { null },
                                frequency = o.optString("frequency", "").ifEmpty { null },
                            )
                        }
                        error = null
                    }.onFailure { error = "Couldn't parse regimens: ${it.message}" }
                },
                onFailure = { error = it.message ?: "Couldn't load medications" },
            )
            // Map regimen_id → latest dose timestamp (best-effort).
            dosesRes.getOrNull()?.let { raw ->
                runCatching {
                    val events = JSONObject(raw).optJSONArray("events") ?: return@runCatching
                    val map = HashMap<String, Long>()
                    for (i in 0 until events.length()) {
                        val e = events.optJSONObject(i) ?: continue
                        val rid = e.optJSONObject("channels")?.optString("regimen_id", "") ?: ""
                        if (rid.isEmpty()) continue
                        val ts = e.optLong("ts_ms", 0L)
                        if (ts > (map[rid] ?: 0L)) map[rid] = ts
                    }
                    lastDoseAt = map
                }
            }
            loading = false
        }
    }

    LaunchedEffect(refreshTick) {
        loading = true
        withContext(Dispatchers.IO) { reload() }
    }

    fun logDose(r: Regimen, doseValue: Double?, doseUnit: String?, skipped: Boolean) {
        scope.launch(Dispatchers.IO) {
            val body = JSONObject()
                .put("name", r.name)
                .put("regimen_id", r.regimenId)
                .put("status", if (skipped) "skipped" else "taken")
            if (!skipped && doseValue != null) body.put("dose_value", doseValue)
            if (!skipped && !doseUnit.isNullOrBlank()) body.put("dose_unit", doseUnit)
            StorageRepository.executeToolJson("log_medication", body.toString())
            withContext(Dispatchers.Main) { onLogMedication(r.name) }
            reload()
        }
    }

    fun discontinue(r: Regimen) {
        scope.launch(Dispatchers.IO) {
            StorageRepository.executeToolJson(
                "discontinue_medication_regimen",
                JSONObject().put("regimen_id", r.regimenId).toString(),
            )
            withContext(Dispatchers.Main) { onToast("Discontinued ${r.name}") }
            reload()
        }
    }

    fun addRegimen(name: String, dose: Double?, unit: String) {
        scope.launch(Dispatchers.IO) {
            val body = JSONObject().put("name", name)
            if (dose != null) body.put("dose_value", dose)
            if (unit.isNotBlank()) body.put("dose_unit", unit)
            StorageRepository.executeToolJson("start_medication_regimen", body.toString())
            withContext(Dispatchers.Main) { onToast("Started $name") }
            reload()
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(
            title = "Medications",
            onBack = onBack,
            action = TopBarAction(label = "Library", onClick = onOpenLibrary),
        )

        Column(
            modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()),
        ) {
            Box(
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
            ) {
                Text(
                    text = "CURRENT MEDICATIONS",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 11.sp,
                    letterSpacing = 2.sp,
                    color = OhdColors.Muted,
                )
            }

            error?.let {
                Text(
                    it, color = OhdColors.Red, fontFamily = OhdBody, fontSize = 13.sp,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                )
            }

            if (!loading && regimens.isEmpty() && error == null) {
                Text(
                    "No active medications. Add one below, or ask CORD to record a prescription.",
                    fontFamily = OhdBody,
                    fontSize = 13.sp,
                    color = OhdColors.Muted,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }

            regimens.forEachIndexed { idx, r ->
                val ts = lastDoseAt[r.regimenId]
                OhdMedLogItem(
                    name = r.name,
                    sub = subtitleFor(r, ts),
                    takenState = if (ts != null && isToday(ts)) TakenState.Taken else TakenState.Pending,
                    onLog = { logDose(r, r.doseValue, r.doseUnit, skipped = false) },
                    onLongPress = { actionTarget = r },
                )
                if (idx < regimens.lastIndex) OhdDivider()
            }

            Spacer(Modifier.height(8.dp))
            Box(
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
            ) {
                OhdButton(
                    label = "+ Add medication",
                    onClick = { addOpen = true },
                    variant = OhdButtonVariant.Ghost,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }

    // Long-press → log actual dose / skip / discontinue.
    actionTarget?.let { r ->
        RegimenActionDialog(
            regimen = r,
            onDismiss = { actionTarget = null },
            onLog = { dose, unit ->
                logDose(r, dose, unit, skipped = false)
                actionTarget = null
            },
            onSkip = {
                logDose(r, null, null, skipped = true)
                actionTarget = null
            },
            onDiscontinue = {
                discontinue(r)
                actionTarget = null
            },
        )
    }

    if (addOpen) {
        AddOnHandDialog(
            onDismiss = { addOpen = false },
            onAdd = { name, dose, unit ->
                addOpen = false
                addRegimen(name, dose, unit)
            },
        )
    }
}

/** An active medication regimen, parsed from list_active_regimens. */
private data class Regimen(
    val regimenId: String,
    val name: String,
    val doseValue: Double?,
    val doseUnit: String?,
    val frequency: String?,
)

private fun subtitleFor(r: Regimen, lastDoseMs: Long?): String {
    val dose = when {
        r.doseValue != null && !r.doseUnit.isNullOrBlank() ->
            "${fmtDose(r.doseValue)} ${r.doseUnit}"
        r.doseValue != null -> fmtDose(r.doseValue)
        else -> null
    }
    val freq = r.frequency
    val regimenLine = listOfNotNull(dose, freq).joinToString(" · ").ifEmpty { "Regimen" }
    return if (lastDoseMs == null) {
        regimenLine
    } else {
        val rel = fmtRelative(lastDoseMs)
        val nice = when {
            rel.endsWith("s ago") -> "just now"
            rel == "1d ago" -> "yesterday"
            else -> rel
        }
        "$regimenLine · last dose $nice"
    }
}

private fun fmtDose(d: Double): String =
    if (d == d.toLong().toDouble()) d.toLong().toString() else d.toString()

private fun isToday(ts: Long): Boolean {
    val now = System.currentTimeMillis()
    return now - ts < 24L * 60L * 60L * 1000L
}

/**
 * Long-press action sheet for a regimen. Records the ACTUAL dose taken
 * (prefilled, editable), with Skip given equal weight to Log dose, plus a
 * destructive Discontinue. No-judgment per project principle.
 */
@Composable
private fun RegimenActionDialog(
    regimen: Regimen,
    onDismiss: () -> Unit,
    onLog: (dose: Double?, unit: String?) -> Unit,
    onSkip: () -> Unit,
    onDiscontinue: () -> Unit,
) {
    var doseText by remember {
        mutableStateOf(regimen.doseValue?.let { fmtDose(it) } ?: "")
    }
    var unit by remember { mutableStateOf(regimen.doseUnit ?: "") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(regimen.name) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(
                    "Record the dose you actually took — adjust it if it differs from " +
                        "the prescription.",
                    fontFamily = OhdBody, fontSize = 13.sp, color = OhdColors.Muted,
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.weight(1f)) {
                        OhdInput(
                            value = doseText, onValueChange = { doseText = it },
                            placeholder = "Dose",
                            keyboardType = androidx.compose.ui.text.input.KeyboardType.Decimal,
                        )
                    }
                    Box(Modifier.weight(1f)) {
                        OhdInput(value = unit, onValueChange = { unit = it }, placeholder = "Unit")
                    }
                }
                TextButton(onClick = onDiscontinue) {
                    Text("Discontinue this medication", color = OhdColors.Red)
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onLog(doseText.trim().toDoubleOrNull(), unit.trim().ifEmpty { null }) }) {
                Text("Log dose")
            }
        },
        dismissButton = {
            TextButton(onClick = onSkip) { Text("Skipped") }
        },
    )
}
