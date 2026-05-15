package com.ohd.connect.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.runtime.LaunchedEffect
import com.ohd.connect.data.EventChannelInput
import com.ohd.connect.data.EventFilter
import com.ohd.connect.data.EventInput
import com.ohd.connect.data.EventVisibility
import com.ohd.connect.data.OhdEvent
import com.ohd.connect.data.OhdScalar
import com.ohd.connect.data.PutEventOutcome
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdCard
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TopBarAction
import com.ohd.connect.ui.icons.visualFor
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Edit-an-event screen — flips the event's source to `manual:android_app`.
 *
 * **Approach.** The uniffi `OhdStorage` surface has no `update_event` /
 * `delete_event` RPC (only `put_event` / `query_events` / grant management
 * etc.). To preserve the audit trail we use a *supersede* pattern:
 *  1. Append a brand-new event with the user's corrected channels and
 *     `source = "manual:android_app"` so the new row carries the
 *     human-edited provenance.
 *  2. Append a thin `audit.event_superseded` pointer event whose channels
 *     carry the original + new ULIDs so a future operator screen can
 *     render the chain. The original row is **kept untouched** in storage
 *     — the audit log already shows the supersede pointer.
 *
 * The user lands here from the pencil affordance on
 * [RecentEventsScreen]; on save we pop back and a snackbar at the
 * activity-level fires via the [onSaved] callback.
 */
@Composable
fun EditEventScreen(
    original: OhdEvent?,
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onSaved: (message: String) -> Unit,
    onError: (message: String) -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        if (original == null) {
            // The event isn't in the recent window we scan client-side
            // (uniffi doesn't expose `eventUlidsIn`). Show a minimal
            // explainer and a back action — better than rendering an
            // empty form the user can't actually save into anything.
            OhdTopBar(title = "Edit event", onBack = onBack)
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(24.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = "Event not in recent window. Open Recent Events again to find it.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 14.sp,
                    color = OhdColors.Muted,
                )
            }
            return@Column
        }

        // Local edit state, one entry per channel + a timestamp field.
        // We key the state to the ULID so navigation back into a different
        // event re-initialises cleanly.
        val initialChannels = remember(original.ulid) {
            original.channels.map { ch ->
                ch.path to (
                    when (val s = ch.scalar) {
                        is OhdScalar.Text -> s.v
                        is OhdScalar.Real -> trimReal(s.v)
                        is OhdScalar.Int -> s.v.toString()
                        is OhdScalar.Bool -> if (s.v) "true" else "false"
                        is OhdScalar.EnumOrdinal -> s.ordinal.toString()
                    }
                )
            }
        }
        // SnapshotStateMap-style: simple parallel list of (path, mutable
        // text). For ~10 channels per event a tiny linear-scan map keeps
        // the screen straightforward without pulling in a Compose-specific
        // map type.
        val edited = remember(original.ulid) {
            mutableStateOf(initialChannels)
        }
        var whenText by remember(original.ulid) {
            mutableStateOf(DT_FORMATTER.format(Date(original.timestampMs)))
        }

        OhdTopBar(
            title = "Edit event",
            onBack = onBack,
            action = TopBarAction(
                label = "Save",
                onClick = {
                    val parsedTs = runCatching {
                        DT_FORMATTER.parse(whenText)?.time
                    }.getOrNull()
                    if (parsedTs == null) {
                        onError("Invalid time — use YYYY-MM-DD HH:MM")
                        return@TopBarAction
                    }
                    val outcome = supersedeEvent(
                        original = original,
                        editedChannelTexts = edited.value,
                        newTimestampMs = parsedTs,
                    )
                    when (outcome) {
                        is SaveOutcome.Saved -> {
                            onSaved("Saved correction. Original kept for audit.")
                            onBack()
                        }
                        is SaveOutcome.Failed -> {
                            onError(outcome.message)
                        }
                    }
                },
            ),
        )

        // Children pulled from the supplementary correlation_id band.
        // `intake.*` rows carry per-nutrient amounts; `composition.*` (and the
        // `custom.composition.*` shadows created before the namespace is
        // canonicalised) carry allergens / additives / ingredients / labels.
        var nutritionChildren by remember(original.ulid) { mutableStateOf<List<OhdEvent>>(emptyList()) }
        var compositionChildren by remember(original.ulid) { mutableStateOf<List<OhdEvent>>(emptyList()) }
        LaunchedEffect(original.ulid) {
            val (n, c) = loadCorrelatedChildren(original)
            nutritionChildren = n
            compositionChildren = c
        }

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            HeaderCard(original)

            if (nutritionChildren.isNotEmpty()) {
                OhdSectionHeader("NUTRITION")
                NutritionChildrenList(nutritionChildren)
            }

            if (compositionChildren.isNotEmpty()) {
                OhdSectionHeader("COMPOSITION")
                CompositionChildrenList(compositionChildren)
            }

            OhdSectionHeader("CHANNELS")
            Column(
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                edited.value.forEachIndexed { index, pair ->
                    val (path, value) = pair
                    val origScalar = original.channels.firstOrNull { it.path == path }?.scalar
                    val keyboardType = when (origScalar) {
                        is OhdScalar.Real, is OhdScalar.Int -> KeyboardType.Number
                        else -> KeyboardType.Text
                    }
                    OhdField(
                        label = path,
                        value = value,
                        onValueChange = { newValue ->
                            val list = edited.value.toMutableList()
                            list[index] = path to newValue
                            edited.value = list
                        },
                        keyboardType = keyboardType,
                    )
                }
            }

            OhdSectionHeader("WHEN")
            OhdField(
                label = "Timestamp",
                value = whenText,
                onValueChange = { whenText = it },
                helper = "YYYY-MM-DD HH:MM (24h, local time)",
            )

            Text(
                text = "Saving will store a corrected copy with source set to " +
                    "'manual:android_app'. The original event remains in the " +
                    "audit log for traceability.",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
        }
    }
}

/** Read-only header card showing the event's icon + type + original time. */
@Composable
private fun HeaderCard(original: OhdEvent) {
    val visual = visualFor(original.eventType)
    val typeLabel = primaryFor(original)
    val origSource = original.source ?: "(unknown source)"
    val origTime = SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.getDefault())
        .format(Date(original.timestampMs))

    OhdCard {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Box(
                modifier = Modifier
                    .size(36.dp)
                    .background(visual.tint.copy(alpha = 0.12f), CircleShape),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = visual.icon,
                    contentDescription = null,
                    tint = visual.tint,
                    modifier = Modifier.size(20.dp),
                )
            }
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    text = typeLabel,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W600,
                    fontSize = 14.sp,
                    color = OhdColors.Ink,
                )
                Text(
                    text = "Logged $origTime",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
                Text(
                    text = "Source: $origSource",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
                Text(
                    text = "ULID …${original.ulid.takeLast(8)}",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
            }
        }
    }
}

// =============================================================================
// Save / supersede pipeline
// =============================================================================

private sealed interface SaveOutcome {
    data class Saved(val newUlid: String) : SaveOutcome
    data class Failed(val message: String) : SaveOutcome
}

/**
 * Build a new `EventInput` from the user's edited channel strings and
 * persist it. On success, also append an `audit.event_superseded` pointer
 * event so a future audit view can follow the chain.
 *
 * Channel value parsing matches the *original* scalar's variant — a Real
 * channel stays Real, an Int stays Int — so we don't accidentally
 * up-cast everything to Text. Parse failures (e.g. user types "five.4"
 * into a numeric field) fall back to the original channel value rather
 * than rejecting the whole save; this keeps the screen forgiving of
 * partial edits.
 */
private fun supersedeEvent(
    original: OhdEvent,
    editedChannelTexts: List<Pair<String, String>>,
    newTimestampMs: Long,
): SaveOutcome {
    val editedTextByPath = editedChannelTexts.associate { it.first to it.second }
    val newChannels: List<EventChannelInput> = original.channels.map { origCh ->
        val edited = editedTextByPath[origCh.path]
        val scalar: OhdScalar = if (edited == null) {
            origCh.scalar
        } else {
            parseScalarFor(origCh.scalar, edited) ?: origCh.scalar
        }
        EventChannelInput(path = origCh.path, scalar = scalar)
    }
    val correctedInput = EventInput(
        timestampMs = newTimestampMs,
        durationMs = original.durationMs,
        eventType = original.eventType,
        channels = newChannels,
        source = "manual:android_app",
        notes = "Edited from ${original.ulid}",
    )

    val newUlid = when (val outcome = StorageRepository.putEvent(correctedInput).getOrNull()) {
        is PutEventOutcome.Committed -> outcome.ulid
        is PutEventOutcome.Pending -> outcome.ulid
        is PutEventOutcome.Error -> return SaveOutcome.Failed(outcome.message)
        null -> return SaveOutcome.Failed("Storage not opened")
    }

    // Best-effort audit pointer — failures here are non-fatal because the
    // corrected event already exists. We swallow the error and surface a
    // success message; the audit pointer is a nice-to-have for the future
    // chain-view, not a hard requirement.
    runCatching {
        StorageRepository.putEvent(
            EventInput(
                timestampMs = System.currentTimeMillis(),
                eventType = "audit.event_superseded",
                channels = listOf(
                    EventChannelInput("original_ulid", OhdScalar.Text(original.ulid)),
                    EventChannelInput("new_ulid", OhdScalar.Text(newUlid)),
                ),
                source = "manual:android_app",
                notes = "Pointer from ${original.ulid} → $newUlid",
            ),
        )
    }

    return SaveOutcome.Saved(newUlid)
}

/**
 * Parse a free-text edit into the same variant as the original scalar.
 *
 * Returns `null` when the input can't be coerced; the caller falls back
 * to the original value (i.e. leaves the field unchanged) rather than
 * rejecting the entire save. Bool channels accept the common literal
 * strings; enum ordinals expect a plain integer string.
 */
private fun parseScalarFor(original: OhdScalar, text: String): OhdScalar? {
    val trimmed = text.trim()
    return when (original) {
        is OhdScalar.Real -> trimmed.toDoubleOrNull()?.let { OhdScalar.Real(it) }
        is OhdScalar.Int -> trimmed.toLongOrNull()?.let { OhdScalar.Int(it) }
        is OhdScalar.Bool -> when (trimmed.lowercase()) {
            "true", "yes", "1" -> OhdScalar.Bool(true)
            "false", "no", "0" -> OhdScalar.Bool(false)
            else -> null
        }
        is OhdScalar.Text -> OhdScalar.Text(trimmed)
        is OhdScalar.EnumOrdinal -> trimmed.toIntOrNull()?.let { OhdScalar.EnumOrdinal(it) }
    }
}

/** Trim a Real to a clean string ("5.4" not "5.4000000000…"). */
private fun trimReal(v: Double): String {
    val rounded = (v * 1000).toLong() / 1000.0
    return if (rounded == rounded.toLong().toDouble()) rounded.toLong().toString()
    else rounded.toString()
}

private val DT_FORMATTER = SimpleDateFormat("yyyy-MM-dd HH:mm", Locale.getDefault())

// =============================================================================
// Children drill-down — `intake.*` + `composition.*` rows that share the
// parent's `correlation_id` channel. Today this is read-only (the parent
// channels stay editable above). Future: tap a child to drill further.
// =============================================================================

private data class CorrelatedChildren(
    val nutrition: List<OhdEvent>,
    val composition: List<OhdEvent>,
)

private fun loadCorrelatedChildren(parent: OhdEvent): Pair<List<OhdEvent>, List<OhdEvent>> {
    val correlationId = parent.channels
        .firstOrNull { it.path == "correlation_id" }
        ?.let { (it.scalar as? OhdScalar.Text)?.v }
        ?: return Pair(emptyList(), emptyList())
    // The Kotlin EventFilter has no channel predicate, so we pull a window of
    // detail-rows around the parent and filter client-side. The intake/
    // composition emit immediately after the parent, so a ±1 h window is
    // comfortably more than enough.
    val fromMs = parent.timestampMs - 60 * 60 * 1_000L
    val toMs = parent.timestampMs + 60 * 60 * 1_000L
    val children = StorageRepository.queryEvents(
        EventFilter(
            fromMs = fromMs,
            toMs = toMs,
            visibility = EventVisibility.NonTopLevelOnly,
            limit = 2_000,
        ),
    ).getOrNull().orEmpty().filter { ev ->
        ev.channels.any { ch ->
            ch.path == "correlation_id" &&
                (ch.scalar as? OhdScalar.Text)?.v == correlationId
        }
    }
    val nutrition = children
        .filter { it.eventType.startsWith("intake.") }
        .sortedBy { it.eventType }
    val composition = children
        .filter {
            it.eventType.startsWith("composition.") ||
                it.eventType.startsWith("custom.composition.")
        }
        .sortedBy { it.eventType }
    return Pair(nutrition, composition)
}

@Composable
private fun NutritionChildrenList(children: List<OhdEvent>) {
    OhdCard {
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            children.forEach { ev ->
                val label = ev.eventType.removePrefix("intake.")
                val v = ev.channels.firstOrNull { it.path == "value" }
                    ?.let { (it.scalar as? OhdScalar.Real)?.v }
                val unit = ev.channels.firstOrNull { it.path == "unit" }
                    ?.let { (it.scalar as? OhdScalar.Text)?.v }
                    .orEmpty()
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text(
                        text = label,
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W500,
                        fontSize = 13.sp,
                        color = OhdColors.Ink,
                    )
                    Text(
                        text = if (v != null) "${trimReal(v)} $unit".trim() else "—",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 13.sp,
                        color = OhdColors.Muted,
                    )
                }
            }
        }
    }
}

@Composable
private fun CompositionChildrenList(children: List<OhdEvent>) {
    // Render as "category: slug" pairs, grouped by category (allergen / trace /
    // additive / label / ingredient / analysis). The event_type encodes both
    // — e.g. `composition.allergen.gluten` or `custom.composition.allergen.gluten`.
    val grouped = children
        .mapNotNull { ev ->
            // Strip optional `custom.` prefix, then `composition.`, leaving
            // `<category>.<slug>`.
            val tail = ev.eventType
                .removePrefix("custom.")
                .removePrefix("composition.")
            val (category, slug) = tail.split('.', limit = 2).let {
                if (it.size == 2) it[0] to it[1] else return@mapNotNull null
            }
            category to slug
        }
        .groupBy({ it.first }, { it.second })
        .toSortedMap()
    OhdCard {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            grouped.forEach { (category, slugs) ->
                Row(modifier = Modifier.fillMaxWidth()) {
                    Text(
                        text = category,
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W500,
                        fontSize = 13.sp,
                        color = OhdColors.Ink,
                        modifier = Modifier.padding(end = 8.dp),
                    )
                    Text(
                        text = slugs.joinToString(", "),
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 13.sp,
                        color = OhdColors.Muted,
                    )
                }
            }
        }
    }
}
