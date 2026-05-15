package com.ohd.connect.ui.screens.settings

import android.content.Context
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.Auth
import com.ohd.connect.data.FormSpec
import com.ohd.connect.data.FormStore
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.screens._shared.CustomMetric
import com.ohd.connect.ui.screens._shared.CustomMetricDialog
import com.ohd.connect.ui.screens._shared.CustomMetricValueType
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import org.json.JSONArray
import org.json.JSONObject

/**
 * Forms & Measurements settings — Pencil `VCokI` "Forms & Measurements" panel.
 *
 * Two sections:
 *  1. **Saved forms** — one row per [FormSpec] persisted via [FormStore].
 *     Tap to edit (via [onEditForm]); the secondary "Fill out" affordance
 *     opens the runtime fill screen (via [onFillForm]). Empty state surfaces
 *     a single muted line explaining the entry point.
 *  2. **Custom measurements** — unchanged from the previous iteration.
 *
 * NavGraph wires [onNewForm] / [onEditForm] / [onFillForm] to the
 * `FormBuilderNew`, `FormBuilderEdit(id)`, `CustomFormFill(id)` routes. The
 * screen itself takes no opinion on what each callback does — it just hands
 * over the id.
 */
@Composable
fun FormsSettingsScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onNewForm: () -> Unit = {},
    onEditForm: (String) -> Unit = {},
    onFillForm: (String) -> Unit = {},
) {
    val ctx = LocalContext.current
    var forms by remember { mutableStateOf(FormStore.load(ctx)) }
    var metrics by remember { mutableStateOf(loadCustomMetrics(ctx)) }
    var showNewMetricDialog by remember { mutableStateOf(false) }
    var renameTarget by remember { mutableStateOf<CustomMetric?>(null) }
    var deleteTarget by remember { mutableStateOf<CustomMetric?>(null) }
    var deleteFormTarget by remember { mutableStateOf<FormSpec?>(null) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Forms & Measurements", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState()),
        ) {
            OhdSectionHeader(text = "SAVED FORMS")

            if (forms.isEmpty()) {
                Text(
                    text = "No custom forms yet — tap “+ New form” below to build one. Forms support sliders, radio (with colour swatches), dropdowns, checkboxes, dates, and more.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    lineHeight = 19.sp,
                    color = OhdColors.Muted,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 14.dp),
                )
            } else {
                forms.forEach { form ->
                    FormRow(
                        form = form,
                        onEdit = { onEditForm(form.id) },
                        onFill = { onFillForm(form.id) },
                        onLongPress = { deleteFormTarget = form },
                    )
                }
            }

            // -----------------------------------------------------------------
            // Custom measurements — app-side metadata only. See the comment on
            // `Auth.customMetricsJson` for the rationale (server-side runtime
            // registry doesn't accept these yet).
            // -----------------------------------------------------------------
            OhdSectionHeader(text = "CUSTOM MEASUREMENTS")

            if (metrics.isEmpty()) {
                Text(
                    text = "No custom measurements yet. Tap “+ New measurement” below to add one — e.g. ankle swelling, peak flow.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    lineHeight = 19.sp,
                    color = OhdColors.Muted,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 14.dp),
                )
            } else {
                metrics.forEach { metric ->
                    val unitSuffix = metric.unit?.let { " · $it" } ?: ""
                    CustomMetricRow(
                        primary = metric.description.ifBlank { metric.name },
                        secondary = "${metric.valueType.label}$unitSuffix",
                        onTap = { renameTarget = metric },
                        onLongPress = { deleteTarget = metric },
                    )
                }
            }

            OhdSectionHeader(text = "CREATE")

            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OhdButton(
                    label = "+ New form",
                    onClick = onNewForm,
                    variant = OhdButtonVariant.Ghost,
                    modifier = Modifier.fillMaxWidth(),
                )
                OhdButton(
                    label = "+ New measurement",
                    onClick = { showNewMetricDialog = true },
                    variant = OhdButtonVariant.Ghost,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }

    // -----------------------------------------------------------------------
    // Dialogs
    // -----------------------------------------------------------------------
    if (showNewMetricDialog) {
        CustomMetricDialog(
            onDismiss = { showNewMetricDialog = false },
            onSave = { newMetric ->
                val updated = metrics + newMetric
                saveCustomMetrics(ctx, updated)
                metrics = updated
                showNewMetricDialog = false
            },
        )
    }

    val rt = renameTarget
    if (rt != null) {
        RenameMetricDialog(
            initial = rt.description,
            onDismiss = { renameTarget = null },
            onSave = { newDescription ->
                val updated = metrics.map { m ->
                    if (m.name == rt.name) m.copy(description = newDescription) else m
                }
                saveCustomMetrics(ctx, updated)
                metrics = updated
                renameTarget = null
            },
        )
    }

    val dt = deleteTarget
    if (dt != null) {
        AlertDialog(
            onDismissRequest = { deleteTarget = null },
            title = {
                Text(
                    text = "Delete measurement?",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 16.sp,
                    color = OhdColors.Ink,
                )
            },
            text = {
                Text(
                    text = "Remove “${dt.description}” from your custom measurements? Existing logged events stay on file.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    color = OhdColors.Ink,
                )
            },
            confirmButton = {
                OhdButton(
                    label = "Delete",
                    onClick = {
                        val updated = metrics.filterNot { it.name == dt.name }
                        saveCustomMetrics(ctx, updated)
                        metrics = updated
                        deleteTarget = null
                    },
                    variant = OhdButtonVariant.Destructive,
                )
            },
            dismissButton = {
                OhdButton(
                    label = "Cancel",
                    onClick = { deleteTarget = null },
                    variant = OhdButtonVariant.Ghost,
                )
            },
            containerColor = OhdColors.Bg,
        )
    }

    val dft = deleteFormTarget
    if (dft != null) {
        AlertDialog(
            onDismissRequest = { deleteFormTarget = null },
            title = {
                Text(
                    text = "Delete form?",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 16.sp,
                    color = OhdColors.Ink,
                )
            },
            text = {
                Text(
                    text = "Remove “${dft.name}”? Already-logged events stay on file.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    color = OhdColors.Ink,
                )
            },
            confirmButton = {
                OhdButton(
                    label = "Delete",
                    onClick = {
                        FormStore.delete(ctx, dft.id)
                        forms = FormStore.load(ctx)
                        deleteFormTarget = null
                    },
                    variant = OhdButtonVariant.Destructive,
                )
            },
            dismissButton = {
                OhdButton(
                    label = "Cancel",
                    onClick = { deleteFormTarget = null },
                    variant = OhdButtonVariant.Ghost,
                )
            },
            containerColor = OhdColors.Bg,
        )
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun FormRow(
    form: FormSpec,
    onEdit: () -> Unit,
    onFill: () -> Unit,
    onLongPress: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(OhdColors.Bg)
            .combinedClickable(onClick = onEdit, onLongClick = onLongPress)
            .padding(horizontal = 16.dp, vertical = 14.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = form.name.ifBlank { "(untitled form)" },
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 14.sp,
                color = OhdColors.Ink,
            )
            Text(
                text = "${form.fields.size} field" + if (form.fields.size == 1) "" else "s",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
        }
        OhdButton(
            label = "Fill out",
            onClick = onFill,
            variant = OhdButtonVariant.Ghost,
        )
    }
}

@Composable
private fun RenameMetricDialog(
    initial: String,
    onDismiss: () -> Unit,
    onSave: (String) -> Unit,
) {
    var text by remember { mutableStateOf(initial) }
    val canSave = text.trim().isNotEmpty()
    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(
                text = "Rename measurement",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 16.sp,
                color = OhdColors.Ink,
            )
        },
        text = {
            OhdField(
                label = "Description",
                value = text,
                onValueChange = { text = it.take(60) },
            )
        },
        confirmButton = {
            OhdButton(
                label = "Save",
                onClick = { onSave(text.trim()) },
                variant = OhdButtonVariant.Primary,
                enabled = canSave,
            )
        },
        dismissButton = {
            OhdButton(
                label = "Cancel",
                onClick = onDismiss,
                variant = OhdButtonVariant.Ghost,
            )
        },
        containerColor = OhdColors.Bg,
    )
}

// =============================================================================
// JSON (de)serialisation for the `custom_metrics_v1` blob.
//
// Schema:
//   { "metrics": [
//       { "namespace": "custom",
//         "name": "ankle_swelling",
//         "description": "Ankle swelling",
//         "value_type": "real" | "int" | "text",
//         "unit": "cm" | null
//       }, ...
//   ] }
//
// v1 is app-side metadata only — see `Auth.customMetricsJson` for the
// rationale. A future server-side endpoint will accept the same JSON and
// register the corresponding event_types via the runtime registry.
// =============================================================================

internal fun loadCustomMetrics(ctx: Context): List<CustomMetric> {
    val raw = Auth.customMetricsJson(ctx) ?: return emptyList()
    return runCatching {
        val root = JSONObject(raw)
        val arr = root.optJSONArray("metrics") ?: return@runCatching emptyList()
        (0 until arr.length()).mapNotNull { i ->
            val obj = arr.optJSONObject(i) ?: return@mapNotNull null
            val vt = when (obj.optString("value_type", "real")) {
                "int" -> CustomMetricValueType.Int
                "text" -> CustomMetricValueType.Text
                else -> CustomMetricValueType.Real
            }
            CustomMetric(
                namespace = obj.optString("namespace", "custom"),
                name = obj.optString("name", ""),
                description = obj.optString("description", ""),
                valueType = vt,
                unit = obj.optString("unit", "").takeIf { it.isNotBlank() },
            )
        }
    }.getOrDefault(emptyList())
}

internal fun saveCustomMetrics(ctx: Context, metrics: List<CustomMetric>) {
    val arr = JSONArray()
    metrics.forEach { m ->
        val obj = JSONObject()
        obj.put("namespace", m.namespace)
        obj.put("name", m.name)
        obj.put("description", m.description)
        obj.put("value_type", m.valueType.storageKey)
        if (m.unit != null) obj.put("unit", m.unit)
        arr.put(obj)
    }
    val root = JSONObject()
    root.put("metrics", arr)
    Auth.saveCustomMetricsJson(ctx, root.toString())
}

// =============================================================================
// CustomMetricRow — clickable list row with both tap (rename) and long-press
// (delete) handlers. OhdListItem only exposes a single onClick, so we
// hand-roll the row layout here using combinedClickable.
// =============================================================================

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun CustomMetricRow(
    primary: String,
    secondary: String,
    onTap: () -> Unit,
    onLongPress: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(OhdColors.Bg)
            .combinedClickable(
                onClick = onTap,
                onLongClick = onLongPress,
            )
            .padding(horizontal = 16.dp, vertical = 14.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(end = 24.dp),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = primary,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 14.sp,
                color = OhdColors.Ink,
            )
            Text(
                text = secondary,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
        }
    }
}
