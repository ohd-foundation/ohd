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
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
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
import com.ohd.connect.data.FieldKind
import com.ohd.connect.data.FieldOption
import com.ohd.connect.data.FormField
import com.ohd.connect.data.FormSpec
import com.ohd.connect.data.FormStore
import com.ohd.connect.data.slugify
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdInput
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdToggle
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TopBarAction
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import java.util.UUID

/**
 * Editable working copy of one [FormField]. Mutable so the row can edit
 * its own state in place without rebuilding the whole field list each
 * keystroke. Compose recomposes the rows via the parent [mutableStateListOf]
 * — we swap items by index when properties change.
 */
private data class FieldDraft(
    val name: String,
    val path: String,
    val pathTouched: Boolean,
    val kind: FieldKind,
    val unit: String,
    val options: List<FieldOption>,
    val min: String,
    val max: String,
    val step: String,
    val required: Boolean,
    val notes: String,
) {
    fun toField(): FormField = FormField(
        name = name.trim(),
        path = path.trim().ifBlank { slugify(name) },
        kind = kind,
        unit = unit.trim().takeIf { it.isNotEmpty() && kind in UNIT_KINDS },
        options = if (kind in OPTION_KINDS) options.filter { it.label.isNotBlank() } else emptyList(),
        min = if (kind == FieldKind.Slider) min.toDoubleOrNull() else null,
        max = if (kind == FieldKind.Slider) max.toDoubleOrNull() else null,
        step = if (kind == FieldKind.Slider) step.toDoubleOrNull() else null,
        required = required,
        notes = notes.trim().takeIf { it.isNotEmpty() },
    )

    companion object {
        fun fromField(f: FormField) = FieldDraft(
            name = f.name,
            path = f.path,
            pathTouched = f.path.isNotBlank() && f.path != slugify(f.name),
            kind = f.kind,
            unit = f.unit.orEmpty(),
            options = f.options,
            min = f.min?.toString().orEmpty(),
            max = f.max?.toString().orEmpty(),
            step = f.step?.toString().orEmpty(),
            required = f.required,
            notes = f.notes.orEmpty(),
        )

        fun blank() = FieldDraft(
            name = "",
            path = "",
            pathTouched = false,
            kind = FieldKind.Real,
            unit = "",
            options = emptyList(),
            min = "",
            max = "",
            step = "",
            required = false,
            notes = "",
        )
    }
}

private val UNIT_KINDS = setOf(FieldKind.Real, FieldKind.Int, FieldKind.Slider)
private val OPTION_KINDS = setOf(FieldKind.Radio, FieldKind.Select, FieldKind.Checkboxes)

/**
 * Form builder — Pencil `NMDCn.png`, spec §4.12.
 *
 * Pass [existing] to open in edit mode; passing `null` (the default)
 * creates a fresh draft. Saving persists via [FormStore.add] /
 * [FormStore.update] and invokes [onSaved] with the resulting spec.
 */
@Composable
fun FormBuilderScreen(
    onBack: () -> Unit,
    onSaved: (FormSpec) -> Unit,
    existing: FormSpec? = null,
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
) {
    val ctx = LocalContext.current
    val formId = remember(existing?.id) { existing?.id ?: UUID.randomUUID().toString() }
    var formName by remember { mutableStateOf(existing?.name.orEmpty()) }
    val drafts = remember {
        mutableStateListOf<FieldDraft>().apply {
            val seed = existing?.fields?.map(FieldDraft::fromField)
                ?: listOf(
                    FieldDraft.fromField(
                        FormField(
                            name = "Glucose",
                            path = "glucose",
                            kind = FieldKind.Real,
                            unit = "mmol/L",
                        ),
                    ),
                )
            addAll(seed)
        }
    }

    fun save() {
        val spec = FormSpec(
            id = formId,
            name = formName.trim().ifBlank { "Untitled form" },
            fields = drafts.map { it.toField() }.filter { it.name.isNotEmpty() },
        )
        if (existing == null) FormStore.add(ctx, spec) else FormStore.update(ctx, spec)
        onSaved(spec)
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(
            title = if (existing == null) "New Form" else "Edit Form",
            onBack = onBack,
            action = TopBarAction(label = "Save", onClick = ::save),
        )

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 20.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            OhdField(
                label = "Form name",
                value = formName,
                onValueChange = { formName = it },
                placeholder = "e.g. Urine strip, Pain score…",
            )

            OhdSectionHeader(text = "FIELDS")

            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                drafts.forEachIndexed { index, draft ->
                    FieldEditor(
                        draft = draft,
                        canMoveUp = index > 0,
                        canMoveDown = index < drafts.lastIndex,
                        onChange = { drafts[index] = it },
                        onMoveUp = {
                            if (index > 0) {
                                val tmp = drafts[index]
                                drafts[index] = drafts[index - 1]
                                drafts[index - 1] = tmp
                            }
                        },
                        onMoveDown = {
                            if (index < drafts.lastIndex) {
                                val tmp = drafts[index]
                                drafts[index] = drafts[index + 1]
                                drafts[index + 1] = tmp
                            }
                        },
                        onDelete = { drafts.removeAt(index) },
                    )
                }
            }

            OhdButton(
                label = "+ Add field",
                onClick = { drafts.add(FieldDraft.blank()) },
                modifier = Modifier.fillMaxWidth(),
                variant = OhdButtonVariant.Ghost,
            )

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                OhdButton(
                    label = "Cancel",
                    onClick = onBack,
                    variant = OhdButtonVariant.Secondary,
                    modifier = Modifier.weight(1f),
                )
                OhdButton(
                    label = "Save",
                    onClick = ::save,
                    variant = OhdButtonVariant.Primary,
                    modifier = Modifier.weight(1f),
                )
            }
        }
    }
}

@Composable
private fun FieldEditor(
    draft: FieldDraft,
    canMoveUp: Boolean,
    canMoveDown: Boolean,
    onChange: (FieldDraft) -> Unit,
    onMoveUp: () -> Unit,
    onMoveDown: () -> Unit,
    onDelete: () -> Unit,
) {
    val shape = RoundedCornerShape(8.dp)
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(OhdColors.Bg, shape)
            .border(BorderStroke(1.dp, OhdColors.Line), shape)
            .padding(horizontal = 14.dp, vertical = 14.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        // Header row — move/delete affordances.
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = draft.name.ifBlank { "(unnamed field)" },
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 14.sp,
                color = OhdColors.Ink,
                modifier = Modifier.weight(1f),
            )
            IconAffordance(
                icon = OhdIcons.ArrowUp,
                enabled = canMoveUp,
                onClick = onMoveUp,
            )
            IconAffordance(
                icon = OhdIcons.ChevronDown,
                enabled = canMoveDown,
                onClick = onMoveDown,
            )
            IconAffordance(
                icon = OhdIcons.Plus, // visual stand-in: rotated cross renders as ×
                enabled = true,
                onClick = onDelete,
                tint = OhdColors.RedDark,
                rotateDeg = 45f,
            )
        }

        OhdField(
            label = "Field name",
            value = draft.name,
            onValueChange = { newName ->
                onChange(
                    draft.copy(
                        name = newName,
                        path = if (draft.pathTouched) draft.path else slugify(newName),
                    ),
                )
            },
            placeholder = "e.g. Colour, Glucose, pH…",
        )

        OhdField(
            label = "Channel path",
            value = draft.path,
            onValueChange = { onChange(draft.copy(path = it, pathTouched = true)) },
            helper = "Used as the channel id under form.<slug>",
            placeholder = "auto from name",
        )

        KindPicker(
            current = draft.kind,
            onPick = { onChange(draft.copy(kind = it)) },
        )

        if (draft.kind in UNIT_KINDS) {
            OhdField(
                label = "Unit (optional)",
                value = draft.unit,
                onValueChange = { onChange(draft.copy(unit = it)) },
                placeholder = "mmol/L, bpm, kg…",
            )
        }

        if (draft.kind == FieldKind.Slider) {
            SliderParamsRow(
                min = draft.min,
                max = draft.max,
                step = draft.step,
                onChange = { newMin, newMax, newStep ->
                    onChange(draft.copy(min = newMin, max = newMax, step = newStep))
                },
            )
        }

        if (draft.kind in OPTION_KINDS) {
            OptionsEditor(
                options = draft.options,
                onChange = { onChange(draft.copy(options = it)) },
            )
        }

        OhdField(
            label = "Helper text (optional)",
            value = draft.notes,
            onValueChange = { onChange(draft.copy(notes = it)) },
            placeholder = "Shown beneath the field at fill time",
        )

        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "Required",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 13.sp,
                color = OhdColors.Ink,
                modifier = Modifier.weight(1f),
            )
            OhdToggle(
                checked = draft.required,
                onCheckedChange = { onChange(draft.copy(required = it)) },
            )
        }
    }
}

@Composable
private fun KindPicker(current: FieldKind, onPick: (FieldKind) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    val shape = RoundedCornerShape(8.dp)
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(
            text = "Widget type",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 13.sp,
            color = OhdColors.Ink,
        )
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
                    text = labelFor(current),
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 14.sp,
                    color = OhdColors.Ink,
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
                FieldKind.values().forEach { kind ->
                    DropdownMenuItem(
                        text = {
                            Text(
                                text = labelFor(kind),
                                fontFamily = OhdBody,
                                fontWeight = FontWeight.W400,
                                fontSize = 14.sp,
                                color = OhdColors.Ink,
                            )
                        },
                        onClick = {
                            onPick(kind)
                            expanded = false
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun SliderParamsRow(
    min: String,
    max: String,
    step: String,
    onChange: (min: String, max: String, step: String) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        ParamCell(
            label = "Min",
            value = min,
            onValueChange = { onChange(it, max, step) },
            modifier = Modifier.weight(1f),
        )
        ParamCell(
            label = "Max",
            value = max,
            onValueChange = { onChange(min, it, step) },
            modifier = Modifier.weight(1f),
        )
        ParamCell(
            label = "Step",
            value = step,
            onValueChange = { onChange(min, max, it) },
            modifier = Modifier.weight(1f),
        )
    }
}

@Composable
private fun ParamCell(
    label: String,
    value: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 12.sp,
            color = OhdColors.Ink,
        )
        OhdInput(
            value = value,
            onValueChange = onValueChange,
            placeholder = "",
            keyboardType = KeyboardType.Number,
        )
    }
}

@Composable
private fun OptionsEditor(
    options: List<FieldOption>,
    onChange: (List<FieldOption>) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(
            text = "Options",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 13.sp,
            color = OhdColors.Ink,
        )
        options.forEachIndexed { index, opt ->
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OhdInput(
                    value = opt.label,
                    onValueChange = { newLabel ->
                        val updated = options.toMutableList()
                        val derivedValue = if (opt.value.isBlank() || opt.value == slugify(opt.label)) {
                            slugify(newLabel)
                        } else opt.value
                        updated[index] = opt.copy(label = newLabel, value = derivedValue)
                        onChange(updated)
                    },
                    placeholder = "Label",
                    modifier = Modifier.weight(1.4f),
                )
                OhdInput(
                    value = opt.color.orEmpty(),
                    onValueChange = { hex ->
                        val updated = options.toMutableList()
                        updated[index] = opt.copy(color = hex.takeIf { it.isNotBlank() })
                        onChange(updated)
                    },
                    placeholder = "#FF0000",
                    modifier = Modifier.weight(1f),
                )
                val swatch = parseHex(opt.color)
                if (swatch != null) {
                    Box(
                        modifier = Modifier
                            .size(20.dp)
                            .background(swatch, CircleShape)
                            .border(BorderStroke(1.dp, OhdColors.Line), CircleShape),
                    )
                }
                IconAffordance(
                    icon = OhdIcons.Plus,
                    enabled = true,
                    onClick = {
                        onChange(options.toMutableList().also { it.removeAt(index) })
                    },
                    tint = OhdColors.RedDark,
                    rotateDeg = 45f,
                )
            }
        }
        OhdButton(
            label = "+ Add option",
            onClick = {
                onChange(options + FieldOption(label = "", value = ""))
            },
            variant = OhdButtonVariant.Ghost,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
private fun IconAffordance(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    enabled: Boolean,
    onClick: () -> Unit,
    tint: Color = OhdColors.Muted,
    rotateDeg: Float = 0f,
) {
    val effectiveTint = if (enabled) tint else tint.copy(alpha = 0.3f)
    Box(
        modifier = Modifier
            .size(32.dp)
            .let { if (enabled) it.clickable { onClick() } else it },
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = effectiveTint,
            modifier = Modifier
                .size(18.dp)
                .let {
                    if (rotateDeg != 0f) it.then(Modifier.padding(0.dp))
                    else it
                },
        )
    }
}

private fun labelFor(kind: FieldKind): String = when (kind) {
    FieldKind.Real -> "Number (decimal)"
    FieldKind.Int -> "Number (integer)"
    FieldKind.Text -> "Text"
    FieldKind.Bool -> "Toggle (on/off)"
    FieldKind.Slider -> "Slider"
    FieldKind.Radio -> "Radio (pick one)"
    FieldKind.Select -> "Dropdown (pick one)"
    FieldKind.Checkboxes -> "Checkboxes (pick many)"
    FieldKind.Date -> "Date (YYYY-MM-DD)"
    FieldKind.Time -> "Time (HH:mm)"
}

internal fun parseHex(hex: String?): Color? {
    if (hex.isNullOrBlank()) return null
    val s = hex.trim().removePrefix("#")
    if (s.length != 6 && s.length != 8) return null
    val v = s.toLongOrNull(16) ?: return null
    return if (s.length == 6) Color((0xFF000000 or v).toInt())
    else Color(v.toInt())
}
