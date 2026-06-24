package com.ohd.connect.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
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
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.ShareKind
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdInput
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * Record a doctor visit — Cases → "+ Record a visit".
 *
 * One screen, one save, three event types. The user fills in who they saw,
 * then optionally lists prescriptions and lab results from that visit; on
 * save we:
 *   1. `record_doctor_visit` → opens a `clinic_visit` case + writes
 *      `clinical.visit`, returns the `case_ulid`.
 *   2. `record_prescription` per drug → writes `clinical.prescription`
 *      AND starts a `medication.regimen_started` (so it shows up in the
 *      Medications screen / list_active_regimens), all tagged `case_id`.
 *   3. `record_lab_result` per result → `clinical.lab_result`, tagged
 *      `case_id`.
 *
 * Everything rides `StorageRepository.executeToolJson` — the same MCP tools
 * an agent calls — so a visit typed here and one Claude records from a
 * photo of a discharge summary land identically. No scanning here; typed
 * entry only (doc-scan is far-future per the plan).
 *
 * Per [[user-clinical-data-sharing]] this is the ground-truth a clinician
 * sees later: we record what the user enters verbatim, no normalising.
 */
@Composable
fun RecordVisitScreen(
    onBack: () -> Unit,
    onSaved: (String) -> Unit,
    onError: (String) -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
) {
    val scope = rememberCoroutineScope()

    // ---- Visit details ----
    var practitioner by remember { mutableStateOf("") }
    var specialty by remember { mutableStateOf("") }
    var facility by remember { mutableStateOf("") }
    var reason by remember { mutableStateOf("") }

    // ---- Staged prescriptions & labs (committed on Save) ----
    var prescriptions by remember { mutableStateOf<List<RxDraft>>(emptyList()) }
    var labs by remember { mutableStateOf<List<LabDraft>>(emptyList()) }

    var saving by remember { mutableStateOf(false) }

    // People you've shared with double as your practitioner contacts (the
    // grant list — no separate person entity yet). Tapping one fills the
    // practitioner field; free-text typing still works. Emergency + agent
    // grants are dropped — they aren't people you'd attribute a visit to.
    var contacts by remember { mutableStateOf<List<String>>(emptyList()) }
    LaunchedEffect(Unit) {
        val loaded = withContext(Dispatchers.IO) {
            // Source 1: people you've shared with (grants) — minus emergency/agent.
            val fromGrants = StorageRepository.listGrants().getOrNull()
                ?.filter {
                    val k = ShareKind.classify(it)
                    k != ShareKind.Emergency && k != ShareKind.Agent
                }
                ?.map { it.granteeLabel }
                ?: emptyList()
            // Source 2: practitioners from past visits — so a doctor you've
            // seen before is one tap away even without a formal share.
            val fromVisits = StorageRepository.executeToolJson(
                "query_events",
                JSONObject().put("event_type", "clinical.visit")
                    .put("visibility", "all").put("limit", 200).toString(),
            ).getOrNull()?.let { raw ->
                runCatching {
                    val events = JSONObject(raw).optJSONArray("events")
                    (0 until (events?.length() ?: 0)).mapNotNull { i ->
                        events!!.optJSONObject(i)?.optJSONObject("channels")
                            ?.optString("practitioner_name", "")?.ifEmpty { null }
                    }
                }.getOrNull() ?: emptyList()
            } ?: emptyList()
            (fromGrants + fromVisits).filter { it.isNotBlank() }.distinct()
        }
        contacts = loaded
    }

    fun save() {
        if (practitioner.isBlank() || saving) return
        saving = true
        scope.launch(Dispatchers.IO) {
            // 1. The visit — opens the case, gives us the case_ulid every
            //    child event tags itself with.
            val visitJson = JSONObject()
                .put("practitioner_name", practitioner.trim())
                .apply {
                    if (specialty.isNotBlank()) put("specialty", specialty.trim())
                    if (facility.isNotBlank()) put("facility", facility.trim())
                    if (reason.isNotBlank()) put("reason", reason.trim())
                }
                .toString()
            val visitRes = StorageRepository.executeToolJson("record_doctor_visit", visitJson)
            val caseId = visitRes.getOrNull()?.let {
                runCatching { JSONObject(it).optString("case_ulid", "") }.getOrDefault("")
            }
            if (visitRes.isFailure || caseId.isNullOrBlank()) {
                withContext(Dispatchers.Main) {
                    saving = false
                    onError(visitRes.exceptionOrNull()?.message ?: "Couldn't record the visit")
                }
                return@launch
            }

            // 2. Prescriptions — each also starts a regimen server-side.
            prescriptions.forEach { rx ->
                val j = JSONObject()
                    .put("case_id", caseId)
                    .put("medication_name", rx.name)
                    .apply {
                        rx.dose.toDoubleOrNull()?.let { put("dose_value", it) }
                        if (rx.unit.isNotBlank()) put("dose_unit", rx.unit)
                        if (rx.frequency.isNotBlank()) put("frequency", rx.frequency)
                    }
                    .toString()
                StorageRepository.executeToolJson("record_prescription", j)
            }

            // 3. Lab results.
            labs.forEach { lab ->
                val j = JSONObject()
                    .put("case_id", caseId)
                    .put("test_name", lab.test)
                    .apply {
                        lab.value.toDoubleOrNull()?.let { put("value", it) }
                            ?: run { if (lab.value.isNotBlank()) put("value_text", lab.value) }
                        if (lab.unit.isNotBlank()) put("unit", lab.unit)
                    }
                    .toString()
                StorageRepository.executeToolJson("record_lab_result", j)
            }

            withContext(Dispatchers.Main) {
                saving = false
                val extras = buildList {
                    if (prescriptions.isNotEmpty()) add("${prescriptions.size} rx")
                    if (labs.isNotEmpty()) add("${labs.size} lab")
                }.joinToString(", ")
                onSaved(
                    "Recorded visit with ${practitioner.trim()}" +
                        if (extras.isNotEmpty()) " · $extras" else "",
                )
                onBack()
            }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Record a visit", onBack = onBack)
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            // ---- Who / where ----
            OhdSectionHeader(text = "VISIT")
            OhdInput(value = practitioner, onValueChange = { practitioner = it }, placeholder = "Practitioner (e.g. Dr. Novák)")
            if (contacts.isNotEmpty()) {
                Text(
                    "From your contacts",
                    fontFamily = OhdBody, fontSize = 11.sp, color = OhdColors.Muted,
                )
                Row(
                    modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    contacts.forEach { name ->
                        OhdButton(
                            label = name,
                            variant = if (practitioner == name) OhdButtonVariant.Primary else OhdButtonVariant.Ghost,
                            onClick = { practitioner = name },
                        )
                    }
                }
            }
            OhdInput(value = specialty, onValueChange = { specialty = it }, placeholder = "Specialty (optional)")
            OhdInput(value = facility, onValueChange = { facility = it }, placeholder = "Facility (optional)")
            OhdInput(value = reason, onValueChange = { reason = it }, placeholder = "Reason for visit (optional)")

            // ---- Prescriptions ----
            OhdSectionHeader(text = "PRESCRIPTIONS")
            if (prescriptions.isEmpty()) {
                Text("None added.", fontFamily = OhdBody, fontSize = 13.sp, color = OhdColors.Muted)
            } else {
                Column(Modifier.fillMaxWidth()) {
                    prescriptions.forEachIndexed { idx, rx ->
                        OhdListItem(
                            primary = rx.name,
                            secondary = listOf(
                                listOf(rx.dose, rx.unit).filter { it.isNotBlank() }.joinToString(" "),
                                rx.frequency,
                            ).filter { it.isNotBlank() }.joinToString(" · ").ifEmpty { null },
                            meta = "Remove",
                            onClick = { prescriptions = prescriptions.filterIndexed { i, _ -> i != idx } },
                        )
                        if (idx < prescriptions.lastIndex) OhdDivider()
                    }
                }
            }
            RxAdder(onAdd = { prescriptions = prescriptions + it })

            // ---- Lab results ----
            OhdSectionHeader(text = "LAB RESULTS")
            if (labs.isEmpty()) {
                Text("None added.", fontFamily = OhdBody, fontSize = 13.sp, color = OhdColors.Muted)
            } else {
                Column(Modifier.fillMaxWidth()) {
                    labs.forEachIndexed { idx, lab ->
                        OhdListItem(
                            primary = lab.test,
                            secondary = listOf(lab.value, lab.unit).filter { it.isNotBlank() }
                                .joinToString(" ").ifEmpty { null },
                            meta = "Remove",
                            onClick = { labs = labs.filterIndexed { i, _ -> i != idx } },
                        )
                        if (idx < labs.lastIndex) OhdDivider()
                    }
                }
            }
            LabAdder(onAdd = { labs = labs + it })

            Box(Modifier.height(8.dp))
            OhdButton(
                label = if (saving) "Saving…" else "Save visit",
                variant = OhdButtonVariant.Primary,
                enabled = practitioner.isNotBlank() && !saving,
                onClick = { save() },
                modifier = Modifier.fillMaxWidth(),
            )
            Box(Modifier.height(24.dp))
        }
    }
}

private data class RxDraft(val name: String, val dose: String, val unit: String, val frequency: String)
private data class LabDraft(val test: String, val value: String, val unit: String)

@Composable
private fun RxAdder(onAdd: (RxDraft) -> Unit) {
    var name by remember { mutableStateOf("") }
    var dose by remember { mutableStateOf("") }
    var unit by remember { mutableStateOf("") }
    var frequency by remember { mutableStateOf("") }
    Column(
        Modifier.fillMaxWidth().padding(top = 4.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        OhdInput(value = name, onValueChange = { name = it }, placeholder = "Medication (e.g. metformin)")
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(Modifier.weight(1f)) {
                OhdInput(value = dose, onValueChange = { dose = it }, placeholder = "Dose", keyboardType = KeyboardType.Decimal)
            }
            Box(Modifier.weight(1f)) {
                OhdInput(value = unit, onValueChange = { unit = it }, placeholder = "Unit (mg)")
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.weight(1f)) {
                OhdInput(value = frequency, onValueChange = { frequency = it }, placeholder = "Frequency (twice daily)")
            }
            OhdButton(
                label = "Add",
                variant = OhdButtonVariant.Ghost,
                enabled = name.isNotBlank(),
                onClick = {
                    onAdd(RxDraft(name.trim(), dose.trim(), unit.trim(), frequency.trim()))
                    name = ""; dose = ""; unit = ""; frequency = ""
                },
            )
        }
    }
}

@Composable
private fun LabAdder(onAdd: (LabDraft) -> Unit) {
    var test by remember { mutableStateOf("") }
    var value by remember { mutableStateOf("") }
    var unit by remember { mutableStateOf("") }
    Column(
        Modifier.fillMaxWidth().padding(top = 4.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        OhdInput(value = test, onValueChange = { test = it }, placeholder = "Test (e.g. HbA1c)")
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.weight(1f)) {
                OhdInput(value = value, onValueChange = { value = it }, placeholder = "Result")
            }
            Box(Modifier.weight(1f)) {
                OhdInput(value = unit, onValueChange = { unit = it }, placeholder = "Unit (%)")
            }
            OhdButton(
                label = "Add",
                variant = OhdButtonVariant.Ghost,
                enabled = test.isNotBlank(),
                onClick = {
                    onAdd(LabDraft(test.trim(), value.trim(), unit.trim()))
                    test = ""; value = ""; unit = ""
                },
            )
        }
    }
}
