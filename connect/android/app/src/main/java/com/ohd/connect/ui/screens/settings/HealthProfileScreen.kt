package com.ohd.connect.ui.screens.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
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
 * Health Profile — Settings → Health profile.
 *
 * The persistent-facts surface: blood type, allergies, conditions, and
 * emergency contacts. Everything is read/written through the MCP tool
 * dispatch (`StorageRepository.executeToolJson`) — the exact same
 * `record_allergy` / `set_blood_type` / `get_health_profile` tools an
 * agent (Claude / Gemini) calls, so the phone and the agent stay in
 * lock-step with zero duplicated logic. No new uniffi surface: the tools
 * already ride the `execute_tool` path on both local and remote storage.
 *
 * Facts are typed per-fact events projected to "current state" server-
 * side; this screen never reasons about event history, it just renders
 * what `get_health_profile` returns and fires record/remove tools.
 */
@Composable
fun HealthProfileScreen(
    onBack: () -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
) {
    val scope = rememberCoroutineScope()
    var refreshTick by remember { mutableStateOf(0) }
    var loading by remember { mutableStateOf(true) }
    var error by remember { mutableStateOf<String?>(null) }

    // Parsed projection from get_health_profile.
    var bloodType by remember { mutableStateOf<String?>(null) }
    var allergies by remember { mutableStateOf<List<Fact>>(emptyList()) }
    var conditions by remember { mutableStateOf<List<Fact>>(emptyList()) }
    var contacts by remember { mutableStateOf<List<Fact>>(emptyList()) }

    // Inline "add" drafts.
    var allergyDraft by remember { mutableStateOf("") }
    var conditionDraft by remember { mutableStateOf("") }
    var contactNameDraft by remember { mutableStateOf("") }
    var contactPhoneDraft by remember { mutableStateOf("") }

    suspend fun reload() {
        val res = StorageRepository.executeToolJson("get_health_profile", "{}")
        withContext(Dispatchers.Main) {
            res.fold(
                onSuccess = { raw ->
                    runCatching {
                        val o = JSONObject(raw)
                        bloodType = o.optJSONObject("blood_type")?.let { bt ->
                            val g = bt.optString("group", "")
                            val rh = bt.optString("rh", "")
                            listOf(g, rhSymbol(rh)).filter { it.isNotEmpty() }.joinToString("")
                        }
                        allergies = parseFacts(o.optJSONArray("allergies"), "allergen")
                        conditions = parseFacts(o.optJSONArray("conditions"), "name")
                        contacts = parseFacts(o.optJSONArray("emergency_contacts"), "name")
                        error = null
                    }.onFailure { error = "Couldn't parse profile: ${it.message}" }
                },
                onFailure = { error = it.message ?: "Couldn't load profile" },
            )
            loading = false
        }
    }

    androidx.compose.runtime.LaunchedEffect(refreshTick) {
        loading = true
        withContext(Dispatchers.IO) { reload() }
    }

    fun callTool(name: String, json: String) {
        scope.launch(Dispatchers.IO) {
            StorageRepository.executeToolJson(name, json)
            reload()
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Health profile", onBack = onBack)
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (error != null) {
                Text(error!!, color = OhdColors.Red, fontFamily = OhdBody, fontSize = 13.sp)
            }

            // ---- Blood type --------------------------------------------
            OhdSectionHeader(text = "BLOOD TYPE")
            Text(
                text = bloodType ?: if (loading) "Loading…" else "Not set",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W600,
                fontSize = 20.sp,
                color = if (bloodType != null) OhdColors.Ink else OhdColors.Muted,
            )
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                for ((label, group, rh) in BLOOD_TYPES) {
                    OhdButton(
                        label = label,
                        variant = OhdButtonVariant.Ghost,
                        onClick = {
                            callTool(
                                "set_blood_type",
                                JSONObject().put("group", group).put("rh", rh).toString(),
                            )
                        },
                    )
                }
            }

            // ---- Allergies ---------------------------------------------
            OhdSectionHeader(text = "ALLERGIES")
            FactList(
                items = allergies,
                empty = "No allergies recorded.",
                onRemove = { f ->
                    callTool("remove_allergy", JSONObject().put("fact_id", f.factId).toString())
                },
            )
            AddRow(
                draft = allergyDraft,
                onDraft = { allergyDraft = it },
                placeholder = "Add allergen (e.g. penicillin)",
                onAdd = {
                    if (allergyDraft.isNotBlank()) {
                        callTool("record_allergy", JSONObject().put("allergen", allergyDraft.trim()).toString())
                        allergyDraft = ""
                    }
                },
            )

            // ---- Conditions --------------------------------------------
            OhdSectionHeader(text = "CONDITIONS")
            FactList(
                items = conditions,
                empty = "No conditions recorded.",
                onRemove = { f ->
                    callTool("resolve_condition", JSONObject().put("fact_id", f.factId).toString())
                },
            )
            AddRow(
                draft = conditionDraft,
                onDraft = { conditionDraft = it },
                placeholder = "Add condition (e.g. asthma)",
                onAdd = {
                    if (conditionDraft.isNotBlank()) {
                        callTool("record_condition", JSONObject().put("name", conditionDraft.trim()).toString())
                        conditionDraft = ""
                    }
                },
            )

            // ---- Emergency contacts ------------------------------------
            OhdSectionHeader(text = "EMERGENCY CONTACTS")
            FactList(
                items = contacts,
                empty = "No emergency contacts.",
                onRemove = { f ->
                    callTool("remove_emergency_contact", JSONObject().put("fact_id", f.factId).toString())
                },
            )
            Row(
                modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(Modifier.weight(1.3f)) {
                    OhdInput(value = contactNameDraft, onValueChange = { contactNameDraft = it }, placeholder = "Name")
                }
                Box(Modifier.weight(1f)) {
                    OhdInput(value = contactPhoneDraft, onValueChange = { contactPhoneDraft = it }, placeholder = "Phone")
                }
                OhdButton(
                    label = "Add",
                    variant = OhdButtonVariant.Ghost,
                    enabled = contactNameDraft.isNotBlank(),
                    onClick = {
                        if (contactNameDraft.isNotBlank()) {
                            callTool(
                                "record_emergency_contact",
                                JSONObject()
                                    .put("name", contactNameDraft.trim())
                                    .put("phone", contactPhoneDraft.trim())
                                    .toString(),
                            )
                            contactNameDraft = ""
                            contactPhoneDraft = ""
                        }
                    },
                )
            }
            Box(Modifier.height(24.dp))
        }
    }
}

/** A current persistent fact, flattened from the tool JSON. */
private data class Fact(val factId: String, val primary: String, val secondary: String?)

private fun parseFacts(arr: org.json.JSONArray?, primaryKey: String): List<Fact> {
    if (arr == null) return emptyList()
    return (0 until arr.length()).mapNotNull { i ->
        val o = arr.optJSONObject(i) ?: return@mapNotNull null
        val primary = o.optString(primaryKey, "").ifEmpty { return@mapNotNull null }
        // Secondary line: severity/relation/phone when present.
        val sev = o.optString("severity", "")
        val rel = o.optString("relation", "")
        val phone = o.optString("phone", "")
        val icd = o.optString("icd10", "")
        val secondary = listOf(sev, icd, rel, phone).filter { it.isNotEmpty() }
            .joinToString(" · ").ifEmpty { null }
        Fact(o.optString("fact_id", primary), primary, secondary)
    }
}

@Composable
private fun FactList(items: List<Fact>, empty: String, onRemove: (Fact) -> Unit) {
    if (items.isEmpty()) {
        Text(empty, fontFamily = OhdBody, fontSize = 13.sp, color = OhdColors.Muted)
        return
    }
    Column(Modifier.fillMaxWidth()) {
        items.forEachIndexed { idx, f ->
            OhdListItem(
                primary = f.primary.replaceFirstChar { it.uppercase() },
                secondary = f.secondary,
                meta = "Remove",
                onClick = { onRemove(f) },
            )
            if (idx < items.lastIndex) OhdDivider()
        }
    }
}

@Composable
private fun AddRow(
    draft: String,
    onDraft: (String) -> Unit,
    placeholder: String,
    onAdd: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.weight(1f)) {
            OhdInput(value = draft, onValueChange = onDraft, placeholder = placeholder)
        }
        OhdButton(label = "Add", variant = OhdButtonVariant.Ghost, enabled = draft.isNotBlank(), onClick = onAdd)
    }
}

private fun rhSymbol(rh: String): String = when (rh) {
    "positive" -> "+"
    "negative" -> "−"
    else -> ""
}

// (label shown, group, rh) — the eight standard ABO/Rh types.
private val BLOOD_TYPES = listOf(
    Triple("O−", "O", "negative"),
    Triple("O+", "O", "positive"),
    Triple("A−", "A", "negative"),
    Triple("A+", "A", "positive"),
    Triple("B−", "B", "negative"),
    Triple("B+", "B", "positive"),
    Triple("AB−", "AB", "negative"),
    Triple("AB+", "AB", "positive"),
)
