#!/usr/bin/env python3
"""Regenerate the cross-runtime metrics outputs from `spec/registry/metrics.toml`.

Outputs:

  * `storage/migrations/018_connect_android_types.sql` — Rust storage core
    seeds. Idempotent INSERT OR IGNORE form. Preserves the hand-written
    section comments + `urine_strip` compact channel layout so that the
    regenerated file is byte-identical to the in-tree migration.
  * `connect/android/app/src/main/java/com/ohd/connect/data/MetricsRegistry.kt`
    — Kotlin object mirroring the registry, consumed by Compose screens.

Run from anywhere:

    python3 storage/tools/regen_migrations.py

The script discovers the repo root by walking up from its own location
(it lives at `storage/tools/`).

Requires Python 3.11+ for `tomllib`. On earlier versions, install `tomli`
and the script will fall back to it.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path
from textwrap import indent
from typing import Any

try:
    import tomllib  # Python 3.11+
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore[no-redef]


# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------

REPO_ROOT = Path(__file__).resolve().parents[2]
TOML_PATH = REPO_ROOT / "spec" / "registry" / "metrics.toml"
SQL_OUT = REPO_ROOT / "storage" / "migrations" / "018_connect_android_types.sql"
KOTLIN_OUT = (
    REPO_ROOT
    / "connect"
    / "android"
    / "app"
    / "src"
    / "main"
    / "java"
    / "com"
    / "ohd"
    / "connect"
    / "data"
    / "MetricsRegistry.kt"
)


# ---------------------------------------------------------------------------
# SQL emit
# ---------------------------------------------------------------------------

SQL_HEADER = """\
-- Connect Android event types.
--
-- Registers the namespaces the Connect Android app writes into directly:
-- `measurement.*` (BP / glucose / weight / temperature / heart rate / SpO2),
-- `medication.*`, `food.*`, `symptom.*`, and `activity.*` (steps / sleep).
--
-- Originally these were rejected by the storage core with `UnknownType`
-- because the seed in `002_std_registry.sql` only registered `std.*` rows
-- (blood_glucose / blood_pressure / heart_rate_resting / body_temperature /
-- medication_dose / symptom / meal / mood). Rather than aliasing every
-- channel back to a canonical std.* shape (which differs in places —
-- e.g. `std.blood_pressure.systolic` vs Connect's `systolic_mmhg`,
-- `std.symptom.severity:int` vs Connect's `severity:real`), we register
-- the Connect shapes as first-class event types in their own namespaces.
-- The two registries coexist; spec/data-model.md documents the canonical
-- mappings for cross-source aggregation later.
--
-- Idempotent — every INSERT is OR IGNORE.
"""

# Special SQL footnotes that appear under specific section headers in the
# in-tree version. Key = section header text.
SECTION_NOTES: dict[str, str] = {
    "symptom.*": (
        "--\n"
        "-- Connect Android's symptom logger uses `symptom.<snake_name>` as the event\n"
        "-- type itself (e.g. `symptom.headache`, `symptom.fatigue`) so the\n"
        "-- per-symptom timelines stay queryable without a name-channel filter. We\n"
        "-- pre-register the 15 default presets; users can also write `symptom.other`\n"
        "-- for free-text input. The shared channel set across all variants:\n"
        "-- `severity` (real, 0–10 NRS), `severity_label` (text), `notes` (text).\n"
        "\n"
        "-- helper macro substitute via repeated blocks. Listed in DefaultSymptoms\n"
        "-- order from `SymptomLogScreen.kt`.\n"
    ),
}


def sql_str(s: str | None) -> str:
    """Render a value as a SQL literal, escaping single quotes."""
    if s is None:
        return "NULL"
    return "'" + s.replace("'", "''") + "'"


def event_type_path(ev: dict[str, Any]) -> str:
    return f"{ev['namespace']}.{ev['name']}"


def emit_section_header(title: str, trailing_blank: bool = True) -> list[str]:
    bar = "-- " + "=" * 75
    out = [bar, f"-- {title}", bar]
    if trailing_blank:
        out.append("")
    return out


def emit_event_type_insert(ev: dict[str, Any]) -> list[str]:
    return [
        "INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)",
        f"VALUES ({sql_str(ev['namespace'])}, {sql_str(ev['name'])}, "
        f"{sql_str(ev['description'])}, {sql_str(ev['sensitivity'])});",
        "",
    ]


def emit_channel_full(ev: dict[str, Any], ch: dict[str, Any]) -> list[str]:
    """Multi-line channel insert (the default form used by all but urine_strip)."""
    name = ch["name"]
    path = ch.get("path", name)
    value_type = ch["value_type"]
    unit = ch.get("unit")
    sensitivity = ch.get("sensitivity", ev["sensitivity"])

    return [
        "INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)",
        f"SELECT id, NULL, {sql_str(name)}, {sql_str(path)}, {sql_str(value_type)}, "
        f"{sql_str(unit)}, {sql_str(sensitivity)}",
        f"FROM event_types WHERE namespace={sql_str(ev['namespace'])} AND name={sql_str(ev['name'])};",
        "",
    ]


def emit_channel_compact(ev: dict[str, Any], channels: list[dict[str, Any]]) -> list[str]:
    """Compact channel insert (urine_strip): pads name+path columns for readability.

    Output shape mirrors the original migration's hand-written form:

        SELECT id, NULL, 'glucose',     'glucose',     'text', ...
        SELECT id, NULL, 'ph',          'ph',          'text', ...

    The name and path columns each get `max(len('<lit>,')) + 2` total width
    (i.e. at least two spaces between columns) so longest names still align.
    """
    name_lits = [sql_str(c["name"]) + "," for c in channels]
    path_lits = [sql_str(c.get("path", c["name"])) + "," for c in channels]
    name_width = max(len(s) for s in name_lits) + 2
    path_width = max(len(s) for s in path_lits) + 2
    out: list[str] = []
    for ch, name_field, path_field in zip(channels, name_lits, path_lits):
        value_type = ch["value_type"]
        unit = ch.get("unit")
        sensitivity = ch.get("sensitivity", ev["sensitivity"])
        out.append(
            "INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)"
        )
        out.append(
            f"SELECT id, NULL, {name_field.ljust(name_width)}{path_field.ljust(path_width)}"
            f"{sql_str(value_type)}, {sql_str(unit)}, {sql_str(sensitivity)} "
            f"FROM event_types WHERE namespace={sql_str(ev['namespace'])} AND name={sql_str(ev['name'])};"
        )
    out.append("")
    return out


def repr_sql(s: str) -> str:
    return sql_str(s)


def emit_pre_comment(text: str, leading_blank: bool) -> list[str]:
    """Render a pre-event-type comment block (already multi-line, no leading '-- ')."""
    out: list[str] = []
    if leading_blank:
        out.append("--")
    for line in text.strip("\n").splitlines():
        out.append(f"-- {line}" if line else "--")
    return out


def emit_event_type(ev: dict[str, Any]) -> list[str]:
    out: list[str] = []
    section_comment = ev.get("sql_section_comment") or event_type_path(ev)
    out.append(f"-- ----------- {section_comment} -----------")
    if "sql_pre_comment" in ev:
        leading_blank = ev.get("sql_pre_comment_leading_blank", True)
        out.extend(emit_pre_comment(ev["sql_pre_comment"], leading_blank))

    out.extend(emit_event_type_insert(ev))

    channels = ev.get("channels", [])
    style = ev.get("sql_channel_style", "full")
    if style == "compact":
        out.extend(emit_channel_compact(ev, channels))
    else:
        for ch in channels:
            # Per-channel pre-comment (currently used by Health Connect adapter
            # channels on weight/temperature).
            if "comment" in ch:
                out.append(f"-- {ch['comment']}")
            out.extend(emit_channel_full(ev, ch))
    return out


def emit_symptom_batch(symptoms: list[dict[str, Any]], shared: list[dict[str, Any]]) -> list[str]:
    """Single batched INSERT for symptom.* event types + shared-channel sweeps."""
    out: list[str] = []
    out.append(
        "INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class) VALUES"
    )
    # Compute padding for `name` and `description` columns (so the in-tree
    # ASCII alignment matches).
    max_name = max(len(sql_str(s["name"])) for s in symptoms)
    max_desc = max(len(sql_str(s["description"])) for s in symptoms)

    last_idx = len(symptoms) - 1
    for i, s in enumerate(symptoms):
        name_lit = sql_str(s["name"])
        desc_lit = sql_str(s["description"])
        name_field = (name_lit + ",").ljust(max_name + 1)
        desc_field = (desc_lit + ",").ljust(max_desc + 1)
        terminator = ";" if i == last_idx else ","
        out.append(
            f"    ({sql_str(s['namespace'])}, {name_field} {desc_field} {sql_str(s['sensitivity'])}){terminator}"
        )
    out.append("")
    out.append("-- Add the three shared channels to every symptom.* type.")
    for ch in shared:
        name = ch["name"]
        path = ch.get("path", name)
        value_type = ch["value_type"]
        unit = ch.get("unit")
        out.append(
            "INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)"
        )
        out.append(
            f"SELECT id, NULL, {sql_str(name)}, {sql_str(path)}, {sql_str(value_type)}, "
            f"{sql_str(unit)}, default_sensitivity_class"
        )
        out.append("FROM event_types WHERE namespace='symptom';")
        out.append("")
    return out


def build_sql(doc: dict[str, Any]) -> str:
    event_types: list[dict[str, Any]] = doc["event_types"]
    shared_symptom_channels = doc.get("symptom_shared_channels", [])

    # Group by section. Section starts with the first ET that carries
    # `sql_section_header`; everything after it (until the next header)
    # belongs to that section.
    lines: list[str] = []
    lines.append(SQL_HEADER.rstrip("\n"))
    lines.append("")

    i = 0
    n = len(event_types)
    while i < n:
        ev = event_types[i]
        header = ev.get("sql_section_header")
        if header is None:
            raise ValueError(f"event_type {event_type_path(ev)} is missing sql_section_header")
        lines.extend(emit_section_header(header, trailing_blank=header not in SECTION_NOTES))

        # Insert the section-level prologue note if there's one (e.g. symptom.*).
        if header in SECTION_NOTES:
            note = SECTION_NOTES[header]
            lines.extend(note.rstrip("\n").splitlines())
            lines.append("")

        # Now walk all event_types until we hit the next sql_section_header.
        section_ets: list[dict[str, Any]] = [ev]
        j = i + 1
        while j < n and "sql_section_header" not in event_types[j]:
            section_ets.append(event_types[j])
            j += 1

        # Emit each event type. Symptom batch is special.
        if ev["namespace"] == "symptom":
            lines.extend(emit_symptom_batch(section_ets, shared_symptom_channels))
        else:
            for k, et in enumerate(section_ets):
                lines.extend(emit_event_type(et))

        i = j

    # Strip trailing blank lines, then re-append a single trailing newline
    # so the file ends exactly like the existing migration.
    while lines and lines[-1] == "":
        lines.pop()
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------
# Kotlin emit
# ---------------------------------------------------------------------------

KOTLIN_HEADER = """\
// Generated by storage/tools/regen_migrations.py from
// spec/registry/metrics.toml. Do not hand-edit — your changes will be
// overwritten on the next regen pass. Edit the TOML instead and re-run:
//
//     python3 storage/tools/regen_migrations.py
//
// Schema version: __SCHEMA_VERSION__
package com.ohd.connect.data

/**
 * One channel inside a metric (a leaf scalar the user writes per event).
 *
 * `path` is the storage-side channel path (often equal to `name`, but for
 * `medication.taken` channels it's prefixed with `med.` per the existing
 * migration). `unit` is the storage unit, not the user-facing one — for
 * user-toggleable display units see [MetricDef.unitOptions].
 */
data class ChannelDef(
    val name: String,
    val valueType: String,
    val unit: String?,
    val path: String,
)

/**
 * One metric (a single event_type the user can record).
 *
 * Generated from `spec/registry/metrics.toml`. `discoverableInQuickLog`
 * surfaces the row in `MeasurementScreen.QuickMeasures`. `unitOptions` and
 * `defaultUnit` are user-facing toggle values (e.g. mmol/L vs mg/dL), not
 * channel storage units.
 */
data class MetricDef(
    val namespace: String,
    val name: String,
    val description: String,
    val sensitivity: String,
    val discoverableInQuickLog: Boolean,
    val unitOptions: List<String>,
    val defaultUnit: String?,
    val channels: List<ChannelDef>,
) {
    /** Fully qualified event-type string used by `StorageRepository.putEvent`. */
    val eventType: String get() = "$namespace.$name"
}

/**
 * Compile-time mirror of the canonical metrics registry.
 *
 * Lookup helpers:
 *  - [byEventType] resolves a flat `"namespace.name"` string (the form used
 *    on the wire / in storage) back to its [MetricDef]. Returns null for
 *    unknown event types (e.g. runtime-registered custom metrics).
 *  - [bySymptomName] short-circuits the `symptom.<name>` lookup used by
 *    SymptomLogScreen, where the caller has the bare snake-case symptom
 *    name (e.g. "headache") rather than the fully qualified event type.
 *  - [quickMeasures] returns the rows that surface in `MeasurementScreen`'s
 *    QUICK MEASURES list, in declaration order.
 */
object MetricsRegistry {
    /** Schema version mirrored from `metrics.toml`. */
    const val SCHEMA_VERSION: Int = __SCHEMA_VERSION__

"""


def kotlin_str(s: str | None) -> str:
    if s is None:
        return "null"
    escaped = s.replace("\\", "\\\\").replace('"', '\\"')
    return f'"{escaped}"'


def kotlin_list_str(xs: list[str]) -> str:
    if not xs:
        return "emptyList()"
    inner = ", ".join(kotlin_str(x) for x in xs)
    return f"listOf({inner})"


def build_kotlin(doc: dict[str, Any]) -> str:
    schema_version = doc.get("schema_version", 1)
    event_types: list[dict[str, Any]] = doc["event_types"]
    shared_symptom_channels = doc.get("symptom_shared_channels", [])

    out: list[str] = []
    header = KOTLIN_HEADER.replace("__SCHEMA_VERSION__", str(schema_version))
    out.append(header.rstrip("\n"))
    out.append("")
    out.append("    val all: List<MetricDef> = listOf(")

    for ev in event_types:
        ns = ev["namespace"]
        name = ev["name"]
        # Build channel list. Symptom types reuse the shared channel set.
        if ns == "symptom":
            channels = list(shared_symptom_channels)
        else:
            channels = ev.get("channels", [])

        out.append("        MetricDef(")
        out.append(f"            namespace = {kotlin_str(ns)},")
        out.append(f"            name = {kotlin_str(name)},")
        out.append(f"            description = {kotlin_str(ev['description'])},")
        out.append(f"            sensitivity = {kotlin_str(ev['sensitivity'])},")
        out.append(
            f"            discoverableInQuickLog = "
            f"{'true' if ev.get('discoverable_in_quick_log', False) else 'false'},"
        )
        out.append(f"            unitOptions = {kotlin_list_str(ev.get('unit_options', []))},")
        out.append(f"            defaultUnit = {kotlin_str(ev.get('default_unit'))},")
        if channels:
            out.append("            channels = listOf(")
            for ch in channels:
                out.append("                ChannelDef(")
                out.append(f"                    name = {kotlin_str(ch['name'])},")
                out.append(f"                    valueType = {kotlin_str(ch['value_type'])},")
                out.append(f"                    unit = {kotlin_str(ch.get('unit'))},")
                out.append(f"                    path = {kotlin_str(ch.get('path', ch['name']))},")
                out.append("                ),")
            out.append("            ),")
        else:
            out.append("            channels = emptyList(),")
        out.append("        ),")

    out.append("    )")
    out.append("")
    out.append("    private val byType: Map<String, MetricDef> = all.associateBy { it.eventType }")
    out.append("")
    out.append("    /** Resolve a flat `\"namespace.name\"` string. */")
    out.append("    fun byEventType(type: String): MetricDef? = byType[type]")
    out.append("")
    out.append("    /**")
    out.append("     * Resolve a bare symptom snake-case name to the matching `symptom.<name>`")
    out.append("     * [MetricDef]. Returns null when the symptom isn't in the canonical")
    out.append("     * registry (caller should fall back to `symptom.other` for free-text).")
    out.append("     */")
    out.append("    fun bySymptomName(name: String): MetricDef? = byType[\"symptom.$name\"]")
    out.append("")
    out.append("    /**")
    out.append("     * Rows surfaced in `MeasurementScreen` QUICK MEASURES — `measurement.*`")
    out.append("     * types with `discoverable_in_quick_log = true` in the TOML.")
    out.append("     */")
    out.append("    fun quickMeasures(): List<MetricDef> =")
    out.append("        all.filter { it.namespace == \"measurement\" && it.discoverableInQuickLog }")
    out.append("}")
    out.append("")
    return "\n".join(out)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main(argv: list[str]) -> int:
    write = True
    out_suffix = ""
    if "--check" in argv:
        write = False
    if "--diff" in argv:
        out_suffix = ".new"

    with open(TOML_PATH, "rb") as f:
        doc = tomllib.load(f)

    sql = build_sql(doc)
    kt = build_kotlin(doc)

    sql_path = SQL_OUT.with_suffix(SQL_OUT.suffix + out_suffix) if out_suffix else SQL_OUT
    kt_path = (
        KOTLIN_OUT.with_suffix(KOTLIN_OUT.suffix + out_suffix) if out_suffix else KOTLIN_OUT
    )

    if write:
        sql_path.write_text(sql)
        kt_path.write_text(kt)
        print(f"wrote {sql_path.relative_to(REPO_ROOT)} ({len(sql)} bytes)")
        print(f"wrote {kt_path.relative_to(REPO_ROOT)} ({len(kt)} bytes)")
    else:
        existing_sql = SQL_OUT.read_text()
        existing_kt = KOTLIN_OUT.read_text() if KOTLIN_OUT.exists() else ""
        ok = True
        if existing_sql != sql:
            ok = False
            print(f"MISMATCH: {SQL_OUT.relative_to(REPO_ROOT)}", file=sys.stderr)
        if existing_kt != kt:
            ok = False
            print(f"MISMATCH: {KOTLIN_OUT.relative_to(REPO_ROOT)}", file=sys.stderr)
        return 0 if ok else 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
