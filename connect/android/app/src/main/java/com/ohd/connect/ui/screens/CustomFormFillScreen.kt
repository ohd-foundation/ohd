package com.ohd.connect.ui.screens

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.EventChannelInput
import com.ohd.connect.data.EventInput
import com.ohd.connect.data.FieldKind
import com.ohd.connect.data.FormField
import com.ohd.connect.data.FormSpec
import com.ohd.connect.data.FormStore
import com.ohd.connect.data.OhdScalar
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.data.slugify
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdInput
import com.ohd.connect.ui.components.OhdToggle
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TopBarAction
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Runtime "fill in this form" screen. Loads the [FormSpec] by id and
 * renders each [FormField] per its kind, emitting one channel per non-empty
 * field on save under `event_type = "form.<slug>"`.
 *
 * Required-field validation flags any missing entries red beneath the
 * field; the Save button stays enabled so the user sees what to fill.
 */
@Composable
fun CustomFormFillScreen(
    formId: String,
    onBack: () -> Unit,
    onLogged: () -> Unit,
    onToast: (String) -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
) {
    val ctx = LocalContext.current
    val spec = remember(formId) {
        FormStore.load(ctx).firstOrNull { it.id == formId }
    }

    if (spec == null) {
        Column(
            modifier = modifier
                .fillMaxSize()
                .background(OhdColors.Bg)
                .padding(contentPadding),
        ) {
            OhdTopBar(title = "Form not found", onBack = onBack)
            Text(
                text = "This form no longer exists. It may have been deleted.",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 13.sp,
                color = OhdColors.Muted,
                modifier = Modifier.padding(16.dp),
            )
        }
        return
    }

    val state = remember(formId) { FormFillState(spec.fields) }
    var showErrors by remember { mutableStateOf(false) }

    fun attemptSave() {
        val missing = spec.fields.filter { f ->
            f.required && state.isEmpty(f.path)
        }
        if (missing.isNotEmpty()) {
            showErrors = true
            onToast("Please fill in ${missing.size} required field${if (missing.size == 1) "" else "s"}")
            return
        }
        val channels = spec.fields.mapNotNull { f -> state.toChannel(f) }
        val formSlug = slugify(spec.name)
        val outcome = StorageRepository.putEvent(
            EventInput(
                timestampMs = System.currentTimeMillis(),
                eventType = "form.$formSlug",
                channels = channels,
            ),
        )
        outcome.fold(
            onSuccess = {
                onToast("Logged ${spec.name}")
                onLogged()
            },
            onFailure = { e -> onToast("Failed to log: ${e.message ?: "unknown error"}") },
        )
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(
            title = spec.name.ifBlank { "Form" },
            onBack = onBack,
            action = TopBarAction(label = "Log", onClick = ::attemptSave),
        )

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 20.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            spec.fields.forEach { field ->
                FieldRenderer(
                    field = field,
                    state = state,
                    showError = showErrors && field.required && state.isEmpty(field.path),
                )
            }

            OhdButton(
                label = "Log",
                onClick = ::attemptSave,
                variant = OhdButtonVariant.Primary,
                modifier = Modifier.fillMaxWidth(),
            )
        }
    }
}

/**
 * In-memory backing store for one fill-out session. Each kind has its own
 * map so we don't have to type-pun a single `Map<String, Any?>`.
 */
private class FormFillState(fields: List<FormField>) {
    val texts = mutableStateMapOf<String, String>()
    val booleans = mutableStateMapOf<String, Boolean>()
    val sliders = mutableStateMapOf<String, Float>()
    val singlePicks = mutableStateMapOf<String, String>()
    val multiPicks = mutableStateMapOf<String, Set<String>>()

    init {
        fields.forEach { f ->
            when (f.kind) {
                FieldKind.Slider -> {
                    val initial = f.min?.toFloat() ?: 0f
                    sliders[f.path] = initial
                }
                FieldKind.Bool -> booleans[f.path] = false
                FieldKind.Checkboxes -> multiPicks[f.path] = emptySet()
                else -> {}
            }
        }
    }

    fun isEmpty(path: String): Boolean {
        if (texts[path]?.isNotBlank() == true) return false
        if (singlePicks[path]?.isNotBlank() == true) return false
        if (multiPicks[path]?.isNotEmpty() == true) return false
        // Bool / Slider are always "set" — they have defaults.
        if (booleans.containsKey(path)) return false
        if (sliders.containsKey(path)) return false
        return true
    }

    fun toChannel(f: FormField): EventChannelInput? {
        val scalar: OhdScalar? = when (f.kind) {
            FieldKind.Real -> texts[f.path]?.toDoubleOrNull()?.let(OhdScalar::Real)
            FieldKind.Int -> texts[f.path]?.toLongOrNull()?.let(OhdScalar::Int)
            FieldKind.Text, FieldKind.Date, FieldKind.Time ->
                texts[f.path]?.takeIf { it.isNotBlank() }?.let(OhdScalar::Text)
            FieldKind.Bool -> OhdScalar.Bool(booleans[f.path] ?: false)
            FieldKind.Slider -> OhdScalar.Real(sliders[f.path]?.toDouble() ?: (f.min ?: 0.0))
            FieldKind.Radio, FieldKind.Select ->
                singlePicks[f.path]?.takeIf { it.isNotBlank() }?.let(OhdScalar::Text)
            FieldKind.Checkboxes -> {
                val picked = multiPicks[f.path].orEmpty()
                if (picked.isEmpty()) null
                else OhdScalar.Text(picked.joinToString(","))
            }
        }
        return scalar?.let { EventChannelInput(path = f.path, scalar = it) }
    }
}

@Composable
private fun FieldRenderer(
    field: FormField,
    state: FormFillState,
    showError: Boolean,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        FieldLabel(field)

        when (field.kind) {
            FieldKind.Real, FieldKind.Int -> {
                OhdInput(
                    value = state.texts[field.path].orEmpty(),
                    onValueChange = { state.texts[field.path] = it },
                    placeholder = field.unit?.let { "value ($it)" } ?: "value",
                    keyboardType = if (field.kind == FieldKind.Int) KeyboardType.Number else KeyboardType.Decimal,
                )
            }
            FieldKind.Text -> {
                OhdInput(
                    value = state.texts[field.path].orEmpty(),
                    onValueChange = { state.texts[field.path] = it },
                    placeholder = "",
                )
            }
            FieldKind.Date -> {
                OhdInput(
                    value = state.texts[field.path].orEmpty(),
                    onValueChange = { state.texts[field.path] = it },
                    placeholder = "YYYY-MM-DD",
                    keyboardType = KeyboardType.Number,
                )
            }
            FieldKind.Time -> {
                OhdInput(
                    value = state.texts[field.path].orEmpty(),
                    onValueChange = { state.texts[field.path] = it },
                    placeholder = "HH:mm",
                    keyboardType = KeyboardType.Number,
                )
            }
            FieldKind.Bool -> {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        text = if (state.booleans[field.path] == true) "Yes" else "No",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 14.sp,
                        color = OhdColors.Ink,
                        modifier = Modifier.weight(1f),
                    )
                    OhdToggle(
                        checked = state.booleans[field.path] == true,
                        onCheckedChange = { state.booleans[field.path] = it },
                    )
                }
            }
            FieldKind.Slider -> SliderField(field, state)
            FieldKind.Radio -> RadioField(field, state)
            FieldKind.Select -> SelectField(field, state)
            FieldKind.Checkboxes -> CheckboxField(field, state)
        }

        if (field.notes != null) {
            Text(
                text = field.notes,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
        }
        if (showError) {
            Text(
                text = "Required",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 12.sp,
                color = OhdColors.RedDark,
            )
        }
    }
}

@Composable
private fun FieldLabel(field: FormField) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = field.name + if (field.unit != null) " · ${field.unit}" else "",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 13.sp,
            color = OhdColors.Ink,
            modifier = Modifier.weight(1f),
        )
        if (field.required) {
            Text(
                text = "*",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 13.sp,
                color = OhdColors.RedDark,
            )
        }
    }
}

@Composable
private fun SliderField(field: FormField, state: FormFillState) {
    val min = (field.min ?: 0.0).toFloat()
    val max = (field.max ?: 100.0).toFloat().coerceAtLeast(min + 1f)
    val step = (field.step ?: 1.0).toFloat().coerceAtLeast(0.0001f)
    val current = state.sliders[field.path] ?: min
    val steps = ((max - min) / step).toInt().minus(1).coerceAtLeast(0)
    Column {
        Slider(
            value = current.coerceIn(min, max),
            onValueChange = { state.sliders[field.path] = it },
            valueRange = min..max,
            steps = steps,
            colors = SliderDefaults.colors(
                thumbColor = OhdColors.Red,
                activeTrackColor = OhdColors.Red,
                inactiveTrackColor = OhdColors.Line,
            ),
        )
        Row(modifier = Modifier.fillMaxWidth()) {
            Text(
                text = formatSlider(current, step),
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 13.sp,
                color = OhdColors.Ink,
                modifier = Modifier.weight(1f),
            )
            Text(
                text = "$min – $max",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
        }
    }
}

private fun formatSlider(v: Float, step: Float): String =
    if (step >= 1f) v.toInt().toString() else String.format("%.2f", v)

@Composable
private fun RadioField(field: FormField, state: FormFillState) {
    val current = state.singlePicks[field.path]
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        field.options.forEach { opt ->
            val selected = current == opt.value
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable { state.singlePicks[field.path] = opt.value }
                    .padding(vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RadioButton(
                    selected = selected,
                    onClick = { state.singlePicks[field.path] = opt.value },
                )
                Text(
                    text = opt.label.ifBlank { opt.value },
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 14.sp,
                    color = OhdColors.Ink,
                    modifier = Modifier.weight(1f),
                )
                val swatch = parseHex(opt.color)
                if (swatch != null) {
                    Box(
                        modifier = Modifier
                            .size(16.dp)
                            .background(swatch, CircleShape)
                            .border(BorderStroke(1.dp, OhdColors.Line), CircleShape),
                    )
                }
            }
        }
    }
}

@Composable
private fun SelectField(field: FormField, state: FormFillState) {
    var expanded by remember { mutableStateOf(false) }
    val current = state.singlePicks[field.path]
    val currentLabel = field.options.firstOrNull { it.value == current }?.label
        ?: if (current.isNullOrBlank()) "Select…" else current
    val shape = RoundedCornerShape(8.dp)
    Box {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(44.dp)
                .background(OhdColors.Bg, shape)
                .border(BorderStroke(1.5.dp, OhdColors.Line), shape)
                .clickable { expanded = true }
                .padding(horizontal = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = currentLabel,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 14.sp,
                color = if (current.isNullOrBlank()) OhdColors.Muted else OhdColors.Ink,
                modifier = Modifier.weight(1f),
            )
            Icon(
                imageVector = OhdIcons.ChevronDown,
                contentDescription = null,
                tint = OhdColors.Muted,
                modifier = Modifier.size(18.dp),
            )
        }
        DropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false },
        ) {
            field.options.forEach { opt ->
                DropdownMenuItem(
                    text = {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            val swatch = parseHex(opt.color)
                            if (swatch != null) {
                                Box(
                                    modifier = Modifier
                                        .size(14.dp)
                                        .background(swatch, CircleShape)
                                        .border(BorderStroke(1.dp, OhdColors.Line), CircleShape),
                                )
                                Box(modifier = Modifier.size(width = 8.dp, height = 1.dp))
                            }
                            Text(
                                text = opt.label.ifBlank { opt.value },
                                fontFamily = OhdBody,
                                fontWeight = FontWeight.W400,
                                fontSize = 14.sp,
                                color = OhdColors.Ink,
                            )
                        }
                    },
                    onClick = {
                        state.singlePicks[field.path] = opt.value
                        expanded = false
                    },
                )
            }
        }
    }
}

@Composable
private fun CheckboxField(field: FormField, state: FormFillState) {
    val current = state.multiPicks[field.path].orEmpty()
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        field.options.forEach { opt ->
            val checked = opt.value in current
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable {
                        state.multiPicks[field.path] =
                            if (checked) current - opt.value else current + opt.value
                    }
                    .padding(vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Checkbox(
                    checked = checked,
                    onCheckedChange = { isChecked ->
                        state.multiPicks[field.path] =
                            if (isChecked) current + opt.value else current - opt.value
                    },
                )
                Text(
                    text = opt.label.ifBlank { opt.value },
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 14.sp,
                    color = OhdColors.Ink,
                    modifier = Modifier.weight(1f),
                )
                val swatch = parseHex(opt.color)
                if (swatch != null) {
                    Box(
                        modifier = Modifier
                            .size(16.dp)
                            .background(swatch, CircleShape)
                            .border(BorderStroke(1.dp, OhdColors.Line), CircleShape),
                    )
                }
            }
        }
    }
}
