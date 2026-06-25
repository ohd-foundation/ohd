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
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Switch
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
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.DueStatus
import com.ohd.connect.data.Schedule
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdInput
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdMedLogItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TakenState
import com.ohd.connect.ui.components.TopBarAction
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * Medications — the tracked-medication surface, split by where each item
 * comes from (plan deep-dancing-teacup.md):
 *
 *  - **ON A TREATMENT PLAN** — regimens tied to a clinical case (`case_id`),
 *    e.g. a drug prescribed at a visit.
 *  - **MY MEDICATIONS** — personal regimens (no case) shown in the one-tap
 *    take-list. The user's own ongoing meds + self-added vitamins.
 *  - **ON HAND** — things the user holds but doesn't take on a cadence
 *    (`on_hand && !quick`, e.g. an EpiPen): inventory, no take button, so it
 *    doesn't clutter daily logging.
 *
 * All of it is real persisted state read from `list_active_regimens` and
 * written through `start_medication_regimen` /
 * `discontinue_medication_regimen` / `log_medication` — the same MCP tools
 * Claude/Gemini call. A regimen started in chat shows up here and vice-versa.
 *
 * Dose logging follows [[project-no-judgment-logging]]: the dialog records
 * the ACTUAL dose taken (prefilled, editable) and gives **Skip** equal
 * billing with **Log dose**. A one-off dose (a pill taken that isn't on any
 * list) logs straight through with no regimen.
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
    // All medication.taken events, newest-first, flattened to Dose. A regimen
    // claims a dose by regimen_id OR (for doses logged by name only — e.g.
    // "took my Mounjaro" via MCP before a regimen existed) by matching name.
    var doses by remember { mutableStateOf<List<Dose>>(emptyList()) }

    var actionTarget by remember { mutableStateOf<Regimen?>(null) }
    var addOpen by remember { mutableStateOf(false) }
    var oneOffOpen by remember { mutableStateOf(false) }

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
                                schedule = o.optString("schedule", "").ifEmpty { null },
                                caseId = o.optString("case_id", "").ifEmpty { null },
                                // Legacy regimens have no flags: default to the
                                // take-list (quick), not inventory.
                                onHand = o.optBoolean("on_hand", false),
                                quick = o.optBoolean("quick", true),
                            )
                        }
                        error = null
                    }.onFailure { error = "Couldn't parse regimens: ${it.message}" }
                },
                onFailure = { error = it.message ?: "Couldn't load medications" },
            )
            // Flatten every medication.taken into a Dose, newest-first.
            dosesRes.getOrNull()?.let { raw ->
                runCatching {
                    val events = JSONObject(raw).optJSONArray("events") ?: return@runCatching
                    val parsed = (0 until events.length()).mapNotNull { i ->
                        val e = events.optJSONObject(i) ?: return@mapNotNull null
                        val ch = e.optJSONObject("channels")
                        Dose(
                            regimenId = ch?.optString("regimen_id", "") ?: "",
                            name = ch?.optString("name", "") ?: "",
                            ts = e.optLong("ts_ms", 0L),
                            doseValue = ch?.takeIf { it.has("dose_value") }?.optDouble("dose_value"),
                            doseUnit = ch?.optString("dose_unit", "")?.ifEmpty { null },
                            skipped = ch?.optBoolean("skipped", false)
                                ?: false || (ch?.optString("status", "") == "skipped"),
                        )
                    }.sortedByDescending { it.ts }
                    doses = parsed
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

    fun oneOffDose(name: String, dose: Double?, unit: String) {
        scope.launch(Dispatchers.IO) {
            val body = JSONObject().put("name", name).put("status", "taken")
            if (dose != null) body.put("dose_value", dose)
            if (unit.isNotBlank()) body.put("dose_unit", unit)
            StorageRepository.executeToolJson("log_medication", body.toString())
            withContext(Dispatchers.Main) { onToast("Logged $name") }
            reload()
        }
    }

    fun discontinue(r: Regimen) {
        scope.launch(Dispatchers.IO) {
            StorageRepository.executeToolJson(
                "discontinue_medication_regimen",
                JSONObject().put("regimen_id", r.regimenId).toString(),
            )
            withContext(Dispatchers.Main) { onToast("Removed ${r.name}") }
            reload()
        }
    }

    fun addRegimen(name: String, dose: Double?, unit: String, frequency: String, onHand: Boolean, quick: Boolean) {
        scope.launch(Dispatchers.IO) {
            val body = JSONObject().put("name", name)
            if (dose != null) body.put("dose_value", dose)
            if (unit.isNotBlank()) body.put("dose_unit", unit)
            if (frequency.isNotBlank()) body.put("frequency", frequency)
            body.put("on_hand", onHand)
            body.put("quick", quick)
            StorageRepository.executeToolJson("start_medication_regimen", body.toString())
            withContext(Dispatchers.Main) { onToast("Added $name") }
            reload()
        }
    }

    // Partition into the three sections. case_id wins (a prescribed med is
    // "on a plan" even if also on-hand); then explicit inventory; else personal.
    val plan = regimens.filter { !it.caseId.isNullOrBlank() }
    val onHand = regimens.filter { it.caseId.isNullOrBlank() && it.onHand && !it.quick }
    val personal = regimens.filter { it.caseId.isNullOrBlank() && !(it.onHand && !it.quick) }

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
            error?.let {
                Text(
                    it, color = OhdColors.Red, fontFamily = OhdBody, fontSize = 13.sp,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }

            if (!loading && regimens.isEmpty() && error == null) {
                Spacer(Modifier.height(8.dp))
                Text(
                    "No medications yet. Add one below, log a one-off dose, or ask CORD to record a prescription.",
                    fontFamily = OhdBody,
                    fontSize = 13.sp,
                    color = OhdColors.Muted,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }

            // ---- ON A TREATMENT PLAN ----
            MedSection(title = "ON A TREATMENT PLAN", regimens = plan, doses = doses,
                onLog = { r -> logDose(r, r.doseValue, r.doseUnit, skipped = false) },
                onOpen = { r -> actionTarget = r })

            // ---- MY MEDICATIONS ----
            MedSection(title = "MY MEDICATIONS", regimens = personal, doses = doses,
                onLog = { r -> logDose(r, r.doseValue, r.doseUnit, skipped = false) },
                onOpen = { r -> actionTarget = r })

            // ---- ON HAND (inventory, no take button) ----
            if (onHand.isNotEmpty()) {
                Spacer(Modifier.height(8.dp))
                OhdSectionHeader(text = "ON HAND")
                onHand.forEachIndexed { idx, r ->
                    OhdListItem(
                        primary = r.name,
                        secondary = inventoryLine(r),
                        meta = "On hand",
                        onClick = { actionTarget = r },
                    )
                    if (idx < onHand.lastIndex) OhdDivider()
                }
            }

            Spacer(Modifier.height(8.dp))
            Column(
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OhdButton(
                    label = "+ Add medication",
                    onClick = { addOpen = true },
                    variant = OhdButtonVariant.Ghost,
                    modifier = Modifier.fillMaxWidth(),
                )
                OhdButton(
                    label = "Log a one-off dose",
                    onClick = { oneOffOpen = true },
                    variant = OhdButtonVariant.Ghost,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }

    // Tap a row → dose history + log actual dose / skip / remove.
    actionTarget?.let { r ->
        // "Extra" when the item isn't currently due (already taken this slot,
        // or not yet due) — logging now is an off-schedule dose.
        val lastTaken = doses.firstOrNull { it.matches(r) && !it.skipped }?.ts
        val st = Schedule.parse(r.schedule).dueStatus(lastTaken, System.currentTimeMillis())
        val extra = st is DueStatus.Taken || st is DueStatus.Upcoming
        RegimenActionDialog(
            regimen = r,
            recent = doses.filter { it.matches(r) }.take(6),
            extra = extra,
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
        AddRegimenDialog(
            onDismiss = { addOpen = false },
            onAdd = { name, dose, unit, frequency, onHandFlag, quickFlag ->
                addOpen = false
                addRegimen(name, dose, unit, frequency, onHandFlag, quickFlag)
            },
        )
    }

    if (oneOffOpen) {
        OneOffDoseDialog(
            onDismiss = { oneOffOpen = false },
            onLog = { name, dose, unit ->
                oneOffOpen = false
                oneOffDose(name, dose, unit)
            },
        )
    }
}

/** A take-listable section (plan / personal). Renders nothing when empty. */
@Composable
private fun MedSection(
    title: String,
    regimens: List<Regimen>,
    doses: List<Dose>,
    onLog: (Regimen) -> Unit,
    onOpen: (Regimen) -> Unit,
) {
    if (regimens.isEmpty()) return
    val now = System.currentTimeMillis()
    Spacer(Modifier.height(8.dp))
    OhdSectionHeader(text = title)
    regimens.forEachIndexed { idx, r ->
        val last = doses.firstOrNull { it.matches(r) }
        // Last *non-skipped* dose satisfies a schedule slot; a skip doesn't.
        val lastTakenMs = doses.firstOrNull { it.matches(r) && !it.skipped }?.ts
        val status = Schedule.parse(r.schedule).dueStatus(lastTakenMs, now)
        OhdMedLogItem(
            name = r.name,
            sub = subtitleFor(r, last, status),
            takenState = takenStateFor(status, last),
            onLog = { onLog(r) },
            onLongPress = { onOpen(r) },
            onOpen = { onOpen(r) },
        )
        if (idx < regimens.lastIndex) OhdDivider()
    }
}

/** Map a schedule [DueStatus] (+ last dose for the unscheduled case) to a button state. */
private fun takenStateFor(status: DueStatus, last: Dose?): TakenState = when (status) {
    is DueStatus.Taken -> TakenState.Taken
    is DueStatus.Upcoming -> TakenState.Upcoming
    is DueStatus.DueNow, is DueStatus.Overdue -> TakenState.Pending
    is DueStatus.Unscheduled ->
        if (last != null && !last.skipped && isToday(last.ts)) TakenState.Taken else TakenState.Pending
}

/** An active medication regimen, parsed from list_active_regimens. */
private data class Regimen(
    val regimenId: String,
    val name: String,
    val doseValue: Double?,
    val doseUnit: String?,
    val frequency: String?,
    val schedule: String?,
    val caseId: String?,
    val onHand: Boolean,
    val quick: Boolean,
)

/** A logged medication.taken event, flattened. */
private data class Dose(
    val regimenId: String,
    val name: String,
    val ts: Long,
    val doseValue: Double?,
    val doseUnit: String?,
    val skipped: Boolean,
) {
    /**
     * A dose belongs to a regimen when their regimen_ids match, OR — for
     * doses logged by name only (e.g. an MCP "took my Mounjaro" recorded
     * before any regimen existed) — when the names match case-insensitively.
     */
    fun matches(r: Regimen): Boolean =
        (regimenId.isNotEmpty() && regimenId == r.regimenId) ||
            (name.isNotEmpty() && name.equals(r.name, ignoreCase = true))

    /** "5 mg" / "5" / null. */
    fun amountLabel(): String? = when {
        doseValue != null && !doseUnit.isNullOrBlank() -> "${fmtDose(doseValue)} $doseUnit"
        doseValue != null -> fmtDose(doseValue)
        else -> null
    }
}

/** "yesterday" / "just now" / "3d ago" from a relative string. */
private fun niceWhen(ts: Long): String {
    val rel = fmtRelative(ts)
    return when {
        rel.endsWith("s ago") -> "just now"
        rel == "1d ago" -> "yesterday"
        else -> rel
    }
}

/**
 * Human label for the stored `schedule` channel. `anchor:<name>` becomes a
 * readable phrase; a cron expr / free text is shown as-is for now (the
 * humanizing/eval engine is a later subsystem).
 */
private fun scheduleLabel(s: String?): String? {
    if (s.isNullOrBlank()) return null
    if (!s.startsWith("anchor:")) return s
    return when (val a = s.removePrefix("anchor:")) {
        "as_needed" -> "as needed"
        "waking" -> "on waking"
        "first_food" -> "with first food"
        "bedtime" -> "at bedtime"
        "each_meal" -> "with each meal"
        else -> "with $a"
    }
}

private fun doseLabel(r: Regimen): String? = when {
    r.doseValue != null && !r.doseUnit.isNullOrBlank() -> "${fmtDose(r.doseValue)} ${r.doseUnit}"
    r.doseValue != null -> fmtDose(r.doseValue)
    else -> null
}

private fun subtitleFor(r: Regimen, last: Dose?, status: DueStatus): String {
    val cadence = scheduleLabel(r.schedule) ?: r.frequency
    val regimenLine = listOfNotNull(doseLabel(r), cadence).joinToString(" · ").ifEmpty { "Regimen" }
    // For a scheduled item the due hint is the most useful tail; for an
    // unscheduled one fall back to the last-dose summary.
    val hint = dueHint(status)
    if (hint != null) return "$regimenLine · $hint"
    return when {
        last == null -> "$regimenLine · no doses logged yet"
        last.skipped -> "$regimenLine · skipped ${niceWhen(last.ts)}"
        else -> {
            val amt = last.amountLabel()
            val tail = if (amt != null) "last $amt · ${niceWhen(last.ts)}" else "last dose ${niceWhen(last.ts)}"
            "$regimenLine · $tail"
        }
    }
}

/** Human due hint for a scheduled item, or null when unscheduled. */
private fun dueHint(status: DueStatus): String? = when (status) {
    is DueStatus.Unscheduled -> null
    is DueStatus.DueNow -> "due now"
    is DueStatus.Overdue -> "overdue ${overdueLabel(status.sinceMs)}"
    is DueStatus.Upcoming -> "next ${fmtClock(status.nextMs)}"
    is DueStatus.Taken -> status.nextMs?.let { "taken · next ${fmtClock(it)}" } ?: "taken ✓"
}

/** "3h" / "2d" — the magnitude of how late a slot is (fmtRelative minus " ago"). */
private fun overdueLabel(sinceMs: Long): String =
    fmtRelative(sinceMs).removeSuffix(" ago").ifBlank { "now" }

/** Short clock label for a near-future slot: "8:00", "tomorrow 8:00", "Thu 8:00". */
private fun fmtClock(ms: Long): String {
    val now = java.util.Calendar.getInstance()
    val t = java.util.Calendar.getInstance().apply { timeInMillis = ms }
    val hm = java.text.SimpleDateFormat("H:mm", java.util.Locale.getDefault()).format(java.util.Date(ms))
    fun day(c: java.util.Calendar) = c.get(java.util.Calendar.YEAR) * 1000 + c.get(java.util.Calendar.DAY_OF_YEAR)
    val diff = day(t) - day(now)
    return when (diff) {
        0 -> hm
        1 -> "tomorrow $hm"
        else -> java.text.SimpleDateFormat("EEE", java.util.Locale.getDefault()).format(java.util.Date(ms)) + " $hm"
    }
}

/** Inventory subtitle for the ON HAND section — dose + schedule, no logging. */
private fun inventoryLine(r: Regimen): String? =
    listOfNotNull(doseLabel(r), scheduleLabel(r.schedule) ?: r.frequency)
        .joinToString(" · ").ifEmpty { null }

private fun fmtDose(d: Double): String =
    if (d == d.toLong().toDouble()) d.toLong().toString() else d.toString()

private fun isToday(ts: Long): Boolean {
    val now = System.currentTimeMillis()
    return now - ts < 24L * 60L * 60L * 1000L
}

/**
 * Action sheet for a regimen. Records the ACTUAL dose taken (prefilled,
 * editable), with Skip given equal weight to Log dose, plus a destructive
 * Remove. No-judgment per project principle.
 */
@Composable
private fun RegimenActionDialog(
    regimen: Regimen,
    recent: List<Dose>,
    extra: Boolean,
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
                scheduleLabel(regimen.schedule)?.let {
                    Text("Schedule: $it", fontFamily = OhdBody, fontSize = 13.sp, color = OhdColors.Muted)
                }
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
                            keyboardType = KeyboardType.Decimal,
                        )
                    }
                    Box(Modifier.weight(1f)) {
                        OhdInput(value = unit, onValueChange = { unit = it }, placeholder = "Unit")
                    }
                }

                // Recent doses — answers "when did I take it and how much".
                if (recent.isNotEmpty()) {
                    Text(
                        "RECENT",
                        fontFamily = OhdBody, fontWeight = FontWeight.W500,
                        fontSize = 10.sp, letterSpacing = 2.sp, color = OhdColors.Muted,
                    )
                    recent.forEach { d ->
                        val label = if (d.skipped) "Skipped" else d.amountLabel() ?: "Dose"
                        Text(
                            "$label · ${niceWhen(d.ts)}",
                            fontFamily = OhdBody, fontSize = 13.sp,
                            color = if (d.skipped) OhdColors.Muted else OhdColors.Ink,
                        )
                    }
                }

                TextButton(onClick = onDiscontinue) {
                    Text("Remove from my medications", color = OhdColors.Red)
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onLog(doseText.trim().toDoubleOrNull(), unit.trim().ifEmpty { null }) }) {
                Text(if (extra) "Log an extra dose" else "Log dose")
            }
        },
        dismissButton = {
            TextButton(onClick = onSkip) { Text("Skipped") }
        },
    )
}

/**
 * Start-a-regimen dialog. Captures a free-text frequency, plus the two
 * tracking flags: **on hand** (the user has it) and **quick** (show in the
 * one-tap take-list). Turn quick off for something you hold but don't take
 * on a cadence (an EpiPen) — it lands in ON HAND instead.
 */
@Composable
private fun AddRegimenDialog(
    onDismiss: () -> Unit,
    onAdd: (name: String, dose: Double?, unit: String, frequency: String, onHand: Boolean, quick: Boolean) -> Unit,
) {
    var name by remember { mutableStateOf("") }
    var doseText by remember { mutableStateOf("") }
    var unit by remember { mutableStateOf("") }
    var frequency by remember { mutableStateOf("") }
    var onHand by remember { mutableStateOf(true) }
    var quick by remember { mutableStateOf(true) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add a medication") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OhdInput(value = name, onValueChange = { name = it }, placeholder = "Name (e.g. Mounjaro)")
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.weight(1f)) {
                        OhdInput(
                            value = doseText, onValueChange = { doseText = it },
                            placeholder = "Dose",
                            keyboardType = KeyboardType.Decimal,
                        )
                    }
                    Box(Modifier.weight(1f)) {
                        OhdInput(value = unit, onValueChange = { unit = it }, placeholder = "Unit (mg)")
                    }
                }
                OhdInput(
                    value = frequency, onValueChange = { frequency = it },
                    placeholder = "Frequency (e.g. weekly, twice daily)",
                )
                ToggleLine("I have this on hand", onHand) { onHand = it }
                ToggleLine("Show in my quick list", quick) { quick = it }
            }
        },
        confirmButton = {
            TextButton(
                enabled = name.isNotBlank(),
                onClick = {
                    onAdd(
                        name.trim(),
                        doseText.trim().replace(',', '.').toDoubleOrNull(),
                        unit.trim(),
                        frequency.trim(),
                        onHand,
                        quick,
                    )
                },
            ) { Text("Add") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

/** Log a single dose of something not on any list (a one-off). */
@Composable
private fun OneOffDoseDialog(
    onDismiss: () -> Unit,
    onLog: (name: String, dose: Double?, unit: String) -> Unit,
) {
    var name by remember { mutableStateOf("") }
    var doseText by remember { mutableStateOf("") }
    var unit by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Log a one-off dose") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text(
                    "A dose of something you're not tracking — logged once, not added to a list.",
                    fontFamily = OhdBody, fontSize = 13.sp, color = OhdColors.Muted,
                )
                OhdInput(value = name, onValueChange = { name = it }, placeholder = "Name (e.g. ibuprofen)")
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Box(Modifier.weight(1f)) {
                        OhdInput(
                            value = doseText, onValueChange = { doseText = it },
                            placeholder = "Dose",
                            keyboardType = KeyboardType.Decimal,
                        )
                    }
                    Box(Modifier.weight(1f)) {
                        OhdInput(value = unit, onValueChange = { unit = it }, placeholder = "Unit (mg)")
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = name.isNotBlank(),
                onClick = {
                    onLog(name.trim(), doseText.trim().replace(',', '.').toDoubleOrNull(), unit.trim())
                },
            ) { Text("Log") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

@Composable
private fun ToggleLine(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, fontFamily = OhdBody, fontSize = 14.sp, color = OhdColors.Ink, modifier = Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = onChange)
    }
}
