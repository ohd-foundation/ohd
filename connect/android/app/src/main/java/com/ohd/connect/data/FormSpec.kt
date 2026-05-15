package com.ohd.connect.data

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

/**
 * Persisted user-authored form. Backed by `Auth.customFormsJson(...)`.
 *
 * `id` is a stable UUID — used by the runtime "fill out" screen and by the
 * builder when navigating in edit mode, so callers don't have to key on
 * `name` (which is mutable and may collide).
 */
data class FormSpec(
    val id: String,
    val name: String,
    val fields: List<FormField>,
)

/**
 * One field on a [FormSpec]. `path` is the channel path emitted under the
 * synthesised `form.<form-slug>` event_type; the runtime form screen reuses
 * dynamic-channel auto-registration shipped in beta27.
 */
data class FormField(
    val name: String,
    val path: String,
    val kind: FieldKind,
    val unit: String? = null,
    val options: List<FieldOption> = emptyList(),
    val min: Double? = null,
    val max: Double? = null,
    val step: Double? = null,
    val required: Boolean = false,
    val notes: String? = null,
)

/**
 * Choice for Radio / Select / Checkboxes. `color` is an optional hex
 * (`#FF0000`) used to render a coloured swatch next to a radio row —
 * intended for urine-strip readings and similar colour-coded scales.
 */
data class FieldOption(
    val label: String,
    val value: String,
    val color: String? = null,
)

/** Widget kind. Each value maps to an [OhdScalar] emission contract. */
enum class FieldKind {
    Real,
    Int,
    Text,
    Bool,
    Slider,
    Radio,
    Select,
    Checkboxes,
    Date,
    Time,
}

/**
 * Persistence facade for the `custom_forms_v1` blob. Tolerates the
 * pre-FormSpec shape (`{forms:[{name,fields:[{name,kind,unit}]}]}`) and
 * migrates forward by minting new UUIDs for legacy entries.
 */
object FormStore {

    private const val SCHEMA_VERSION = 2

    /** Load all saved forms. Returns empty when nothing is persisted. */
    fun load(ctx: Context): List<FormSpec> {
        val raw = Auth.customFormsJson(ctx) ?: return emptyList()
        return runCatching {
            val root = JSONObject(raw)
            val arr = root.optJSONArray("forms") ?: return@runCatching emptyList()
            (0 until arr.length()).mapNotNull { i ->
                val obj = arr.optJSONObject(i) ?: return@mapNotNull null
                parseForm(obj)
            }
        }.getOrElse { emptyList() }
    }

    fun save(ctx: Context, specs: List<FormSpec>) {
        val arr = JSONArray()
        specs.forEach { arr.put(encodeForm(it)) }
        val root = JSONObject()
        root.put("schema", SCHEMA_VERSION)
        root.put("forms", arr)
        Auth.saveCustomFormsJson(ctx, root.toString())
    }

    fun add(ctx: Context, spec: FormSpec) {
        save(ctx, load(ctx) + spec)
    }

    fun update(ctx: Context, spec: FormSpec) {
        val existing = load(ctx)
        val replaced = existing.map { if (it.id == spec.id) spec else it }
        // If the id wasn't in the list, treat as add — caller bug, but no reason to crash.
        save(ctx, if (replaced == existing) existing + spec else replaced)
    }

    fun delete(ctx: Context, id: String) {
        save(ctx, load(ctx).filterNot { it.id == id })
    }

    // ----------------------------------------------------------------------
    // JSON helpers
    // ----------------------------------------------------------------------

    private fun parseForm(obj: JSONObject): FormSpec {
        val id = obj.optString("id", "").ifBlank { UUID.randomUUID().toString() }
        val name = obj.optString("name", "")
        val fieldsArr = obj.optJSONArray("fields") ?: JSONArray()
        val fields = (0 until fieldsArr.length()).mapNotNull { j ->
            val f = fieldsArr.optJSONObject(j) ?: return@mapNotNull null
            parseField(f)
        }
        return FormSpec(id = id, name = name, fields = fields)
    }

    private fun parseField(f: JSONObject): FormField {
        val name = f.optString("name", "")
        // The legacy shape used `"kind"` for what was effectively a free
        // string ("number" / "text"). Map those to the new enum so old
        // installs render without dataloss.
        val kindRaw = f.optString("kind", "Text")
        val kind = parseKind(kindRaw)
        val path = f.optString("path", "").ifBlank { slugify(name) }
        val unit = f.optString("unit", "").takeIf { it.isNotBlank() }
        val notes = f.optString("notes", "").takeIf { it.isNotBlank() }
        val required = f.optBoolean("required", false)
        val min = if (f.has("min")) f.optDouble("min") else null
        val max = if (f.has("max")) f.optDouble("max") else null
        val step = if (f.has("step")) f.optDouble("step") else null
        val optsArr = f.optJSONArray("options") ?: JSONArray()
        val options = (0 until optsArr.length()).mapNotNull { i ->
            val o = optsArr.optJSONObject(i) ?: return@mapNotNull null
            FieldOption(
                label = o.optString("label", ""),
                value = o.optString("value", o.optString("label", "")),
                color = o.optString("color", "").takeIf { it.isNotBlank() },
            )
        }
        return FormField(
            name = name,
            path = path,
            kind = kind,
            unit = unit,
            options = options,
            min = min,
            max = max,
            step = step,
            required = required,
            notes = notes,
        )
    }

    private fun parseKind(raw: String): FieldKind {
        // Try the v2 enum names first, then fall through to the v1 string
        // tokens. Anything unrecognised becomes Text so the user still sees
        // their field with a sensible default.
        FieldKind.values().firstOrNull { it.name.equals(raw, ignoreCase = true) }
            ?.let { return it }
        return when (raw.lowercase()) {
            "number", "real", "float", "double" -> FieldKind.Real
            "int", "integer" -> FieldKind.Int
            "bool", "boolean", "toggle" -> FieldKind.Bool
            "enum" -> FieldKind.Radio
            else -> FieldKind.Text
        }
    }

    private fun encodeForm(spec: FormSpec): JSONObject {
        val obj = JSONObject()
        obj.put("id", spec.id)
        obj.put("name", spec.name)
        val fieldsArr = JSONArray()
        spec.fields.forEach { fieldsArr.put(encodeField(it)) }
        obj.put("fields", fieldsArr)
        return obj
    }

    private fun encodeField(field: FormField): JSONObject {
        val obj = JSONObject()
        obj.put("name", field.name)
        obj.put("path", field.path)
        obj.put("kind", field.kind.name)
        if (field.unit != null) obj.put("unit", field.unit)
        if (field.notes != null) obj.put("notes", field.notes)
        if (field.required) obj.put("required", true)
        if (field.min != null) obj.put("min", field.min)
        if (field.max != null) obj.put("max", field.max)
        if (field.step != null) obj.put("step", field.step)
        if (field.options.isNotEmpty()) {
            val arr = JSONArray()
            field.options.forEach { o ->
                val oo = JSONObject()
                oo.put("label", o.label)
                oo.put("value", o.value)
                if (o.color != null) oo.put("color", o.color)
                arr.put(oo)
            }
            obj.put("options", arr)
        }
        return obj
    }
}

/**
 * Slugify a free-text label into a snake-case identifier. Keeps ASCII
 * letters/digits, lowercases, and collapses runs of other characters to a
 * single underscore. Empty input becomes `"field"` so callers always get a
 * usable path token.
 */
fun slugify(text: String): String {
    val out = StringBuilder()
    var lastWasUnderscore = true
    for (ch in text) {
        val c = when {
            ch in 'a'..'z' || ch in '0'..'9' -> ch
            ch in 'A'..'Z' -> ch + ('a' - 'A')
            else -> '_'
        }
        if (c == '_') {
            if (!lastWasUnderscore) out.append('_')
            lastWasUnderscore = true
        } else {
            out.append(c)
            lastWasUnderscore = false
        }
    }
    return out.toString().trim('_').ifEmpty { "field" }
}
