package com.ohd.connect.ui.screens

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.AuditEntry
import com.ohd.connect.data.AuditFilter
import com.ohd.connect.data.MetricsRegistry
import com.ohd.connect.data.EventFilter
import com.ohd.connect.data.EventVisibility
import com.ohd.connect.data.OhdEvent
import com.ohd.connect.data.OhdScalar
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdSegmentedTimeRange
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TimeRange
import com.ohd.connect.ui.icons.EventVisual
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.icons.visualFor
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale
import kotlin.math.abs

/**
 * Recent Events — Pencil `H06Ms`, spec §4.3.
 *
 * Renders the last 50 events as a timeline of richer rows: a circular
 * type-coloured icon, a human-readable primary line (channel summary), a
 * secondary line (source + device or correlation status), a right-side
 * vertical column with the relative timestamp and a pencil affordance
 * that opens [EditEventScreen] for that event.
 *
 * Pulls real rows via [StorageRepository.queryEvents] /
 * [StorageRepository.auditQuery] when the storage handle is open;
 * otherwise falls back to [StubData.recentEventsSample] so the screen
 * renders during onboarding / before storage is wired.
 */
@Composable
fun RecentEventsScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onEdit: (ulid: String) -> Unit = {},
) {
    // Selected time range (Day/Week/Month — Year is mapped to Month) and the
    // active event-type filter ("All" when null).
    var range by remember { mutableStateOf(TimeRange.Today) }
    var selectedType by remember { mutableStateOf<String?>(null) }

    var events by remember { mutableStateOf<List<OhdEvent>>(emptyList()) }
    // Distinct event types present within the current range — drives the chips.
    var types by remember { mutableStateOf<List<String>>(emptyList()) }
    var fallback by remember { mutableStateOf<List<DisplayRow>>(emptyList()) }
    var loaded by remember { mutableStateOf(false) }
    // correlation_id → summed nutrition from intake.* children. Lets the
    // food.eaten / consumption_* row show "320 kcal · 18g carbs" even though
    // the parent event itself only carries name + grams since the beta28
    // intake split.
    var foodNutrition by remember { mutableStateOf<Map<String, FoodTotals>>(emptyMap()) }

    // Re-query whenever the range or the type filter changes. queryEvents is
    // synchronous but pushed to IO to keep the main thread free.
    LaunchedEffect(range, selectedType) {
        val fromMs = rangeStartMs(range)
        val result = withContext(Dispatchers.IO) {
            // Full range (no type filter) — drives the chip set.
            val all = StorageRepository.queryEvents(
                EventFilter(
                    fromMs = fromMs,
                    limit = 2_000L,
                    visibility = EventVisibility.TopLevelOnly,
                ),
            ).getOrNull().orEmpty()
            // Filtered list — by selected type when one is active.
            val filtered = if (selectedType != null) {
                all.filter { it.eventType == selectedType }
            } else {
                all
            }
            val nutrition = aggregateFoodNutrition(filtered)
            Triple(all, filtered, nutrition)
        }
        types = result.first.map { it.eventType }.distinct().sorted()
        events = result.second
        foodNutrition = result.third
        loaded = true
        // The type filter may point at a type no longer present after a
        // range change — fall back to "All" so the list isn't empty.
        if (selectedType != null && selectedType !in types) {
            selectedType = null
        }
    }

    LaunchedEffect(Unit) {
        // Stub / audit fallback only matters on a brand-new install with no
        // structured events at all; computed once.
        fallback = withContext(Dispatchers.IO) { loadFallbackRows() }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "History", onBack = onBack)

        // ---- Controls: range selector + event-type filter chips ----------
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 4.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            OhdSegmentedTimeRange(selected = range, onSelect = { range = it })

            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                FilterChip(
                    label = "All",
                    selected = selectedType == null,
                    onClick = { selectedType = null },
                )
                types.forEach { t ->
                    FilterChip(
                        label = humanizeEventType(t),
                        selected = selectedType == t,
                        onClick = { selectedType = t },
                    )
                }
            }
        }

        // ---- Optional chart (single scalar type selected) ----------------
        val numericPoints = remember(events, selectedType) {
            if (selectedType != null) numericSeries(events) else emptyList()
        }
        if (selectedType != null && numericPoints.isNotEmpty()) {
            EventChart(
                points = numericPoints,
                range = range,
                rangeStart = rangeStartMs(range),
            )
        }

        OhdSectionHeader(
            if (selectedType != null) humanizeEventType(selectedType!!).uppercase(Locale.getDefault())
            else "ALL ENTRIES",
        )

        if (events.isNotEmpty()) {
            // Build a quick lookup of finished consumption pairs so we can
            // annotate the "started" row with the duration of its finished
            // counterpart, and skip duplicate "finished" rows visually.
            val finishedByCorr = remember(events) {
                events.filter { it.eventType == "food.consumption_finished" }
                    .mapNotNull { ev ->
                        val cid = ev.channels.firstOrNull { it.path == "correlation_id" }
                            ?.scalar?.let { (it as? OhdScalar.Text)?.v }
                        cid?.let { it to ev }
                    }
                    .toMap()
            }
            LazyColumn(modifier = Modifier.fillMaxSize()) {
                itemsIndexed(events) { idx, ev ->
                    EventRow(
                        event = ev,
                        finishedPair = finishedByCorr[ev.correlationId()],
                        nutrition = foodNutrition[ev.correlationId()],
                        onEdit = { onEdit(ev.ulid) },
                    )
                    if (idx < events.lastIndex) OhdDivider()
                }
            }
        } else if (loaded && types.isEmpty()) {
            // No structured events at all — show the audit/stub fallback.
            LazyColumn(modifier = Modifier.fillMaxSize()) {
                itemsIndexed(fallback) { idx, row ->
                    OhdListItem(
                        primary = row.primary,
                        secondary = row.secondary,
                        meta = row.meta,
                    )
                    if (idx < fallback.lastIndex) OhdDivider()
                }
            }
        } else {
            // Range/filter combination has no matching events.
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.TopCenter,
            ) {
                Text(
                    text = "No entries in this range.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    color = OhdColors.Muted,
                    modifier = Modifier.padding(top = 32.dp),
                )
            }
        }
    }
}

/** Start-of-range timestamp (ms), local timezone. Year folds into Month. */
private fun rangeStartMs(range: TimeRange): Long {
    val cal = Calendar.getInstance()
    when (range) {
        TimeRange.Today -> {
            cal.set(Calendar.HOUR_OF_DAY, 0)
            cal.set(Calendar.MINUTE, 0)
            cal.set(Calendar.SECOND, 0)
            cal.set(Calendar.MILLISECOND, 0)
        }
        TimeRange.Week -> cal.add(Calendar.DAY_OF_YEAR, -7)
        TimeRange.Month, TimeRange.Year -> cal.add(Calendar.DAY_OF_YEAR, -30)
    }
    return cal.timeInMillis
}

/**
 * Humanize an event type for a chip / header label: take the part after the
 * last `.`, replace `_` with spaces, capitalise the first letter.
 * e.g. `measurement.heart_rate` → "Heart rate", `food.eaten` → "Eaten".
 */
internal fun humanizeEventType(eventType: String): String {
    val tail = eventType.substringAfterLast('.')
    val spaced = tail.replace('_', ' ').trim()
    if (spaced.isEmpty()) return eventType
    return spaced.replaceFirstChar { it.uppercase() }
}

// =============================================================================
// Chart — pure Canvas drawing
// =============================================================================

/** One scalar sample for the chart. */
private data class ScalarPoint(val timestampMs: Long, val value: Double)

/**
 * Extract the numeric series for a list of (single-type) events. Picks the
 * first channel per event whose scalar is [OhdScalar.Real] or
 * [OhdScalar.Int]. Returns empty when the type is not scalar.
 */
private fun numericSeries(events: List<OhdEvent>): List<ScalarPoint> =
    events.mapNotNull { ev ->
        val v = ev.channels.firstNotNullOfOrNull { ch ->
            when (val s = ch.scalar) {
                is OhdScalar.Real -> s.v
                is OhdScalar.Int -> s.v.toDouble()
                else -> null
            }
        }
        v?.let { ScalarPoint(ev.timestampMs, it) }
    }.sortedBy { it.timestampMs }

/**
 * Scalar chart drawn above the list. Day range → line + dots over the day.
 * Week/Month → per-day min/max bars with an avg tick (Week overlays raw
 * points faintly). ~160 dp tall, pure [Canvas].
 */
@Composable
private fun EventChart(
    points: List<ScalarPoint>,
    range: TimeRange,
    rangeStart: Long,
) {
    val minV = points.minOf { it.value }
    val maxV = points.maxOf { it.value }
    val span = (maxV - minV).takeIf { it > 0.0001 } ?: 1.0
    val lo = minV - span * 0.1
    val hi = maxV + span * 0.1

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 4.dp),
    ) {
        Row(modifier = Modifier.fillMaxWidth()) {
            // Y axis labels.
            Column(
                modifier = Modifier.height(160.dp),
                verticalArrangement = Arrangement.SpaceBetween,
            ) {
                ChartAxisLabel(fmtChartNumber(hi))
                ChartAxisLabel(fmtChartNumber((hi + lo) / 2))
                ChartAxisLabel(fmtChartNumber(lo))
            }
            Spacer(Modifier.size(6.dp))
            Canvas(
                modifier = Modifier
                    .weight(1f)
                    .height(160.dp),
            ) {
                val w = size.width
                val h = size.height
                fun yOf(v: Double): Float =
                    (h - ((v - lo) / (hi - lo)).toFloat() * h).coerceIn(0f, h)

                // Baseline grid: top / mid / bottom.
                listOf(0f, h / 2f, h).forEach { gy ->
                    drawLine(
                        color = OhdColors.Line,
                        start = Offset(0f, gy),
                        end = Offset(w, gy),
                        strokeWidth = 1f,
                    )
                }

                if (range == TimeRange.Today) {
                    // ---- Day: line + dot chart across 24h ----
                    val dayMs = 24f * 60f * 60f * 1000f
                    fun xOf(ts: Long): Float =
                        (((ts - rangeStart).toFloat() / dayMs)).coerceIn(0f, 1f) * w

                    val pts = points.map { Offset(xOf(it.timestampMs), yOf(it.value)) }
                    for (i in 0 until pts.size - 1) {
                        drawLine(
                            color = OhdColors.Red,
                            start = pts[i],
                            end = pts[i + 1],
                            strokeWidth = 2.5f,
                        )
                    }
                    pts.forEach { p ->
                        drawCircle(color = OhdColors.Red, radius = 4f, center = p)
                        drawCircle(color = OhdColors.Bg, radius = 1.8f, center = p)
                    }
                } else {
                    // ---- Week/Month: per-day min/max bars + avg tick ----
                    val buckets = bucketByDay(points)
                    val dayStarts = buckets.keys.sorted()
                    if (dayStarts.isNotEmpty()) {
                        val firstDay = dayStarts.first()
                        val lastDay = dayStarts.last()
                        val daySpan = (lastDay - firstDay).toFloat()
                            .takeIf { it > 0f } ?: 1f
                        val barW = (w / (dayStarts.size * 1.6f)).coerceIn(3f, 26f)

                        // Week: faint raw points behind the bars.
                        if (range == TimeRange.Week) {
                            val totalSpan = (lastDay + 86_400_000L - firstDay)
                                .toFloat().takeIf { it > 0f } ?: 1f
                            points.forEach { p ->
                                val x = ((p.timestampMs - firstDay).toFloat()
                                    / totalSpan).coerceIn(0f, 1f) * w
                                drawCircle(
                                    color = OhdColors.Muted.copy(alpha = 0.25f),
                                    radius = 2.5f,
                                    center = Offset(x, yOf(p.value)),
                                )
                            }
                        }

                        dayStarts.forEach { day ->
                            val vs = buckets.getValue(day)
                            val x = if (dayStarts.size == 1) {
                                w / 2f
                            } else {
                                ((day - firstDay).toFloat() / daySpan)
                                    .coerceIn(0f, 1f) * (w - barW) + barW / 2f
                            }
                            val yMin = yOf(vs.min())
                            val yMax = yOf(vs.max())
                            val yAvg = yOf(vs.average())
                            // Min-max bar.
                            drawLine(
                                color = OhdColors.Red.copy(alpha = 0.45f),
                                start = Offset(x, yMin),
                                end = Offset(x, yMax),
                                strokeWidth = barW,
                            )
                            // Avg tick.
                            drawLine(
                                color = OhdColors.Ink,
                                start = Offset(x - barW / 2f, yAvg),
                                end = Offset(x + barW / 2f, yAvg),
                                strokeWidth = 2f,
                            )
                        }
                    }
                }
                // Stroke frame.
                drawRect(
                    color = OhdColors.Line,
                    topLeft = Offset(0f, 0f),
                    size = size,
                    style = Stroke(width = 1f),
                )
            }
        }
        // X axis labels.
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(start = 36.dp, top = 4.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            chartXLabels(range, rangeStart).forEach { ChartAxisLabel(it) }
        }
    }
}

@Composable
private fun ChartAxisLabel(text: String) {
    Text(
        text = text,
        fontFamily = OhdBody,
        fontWeight = FontWeight.W400,
        fontSize = 10.sp,
        color = OhdColors.Muted,
    )
}

/** Group scalar points into buckets keyed by local-day start (ms). */
private fun bucketByDay(points: List<ScalarPoint>): Map<Long, List<Double>> {
    val cal = Calendar.getInstance()
    return points.groupBy { p ->
        cal.timeInMillis = p.timestampMs
        cal.set(Calendar.HOUR_OF_DAY, 0)
        cal.set(Calendar.MINUTE, 0)
        cal.set(Calendar.SECOND, 0)
        cal.set(Calendar.MILLISECOND, 0)
        cal.timeInMillis
    }.mapValues { (_, ps) -> ps.map { it.value } }
}

/** X-axis tick labels for the chart frame. */
private fun chartXLabels(range: TimeRange, rangeStart: Long): List<String> = when (range) {
    TimeRange.Today -> listOf("00:00", "12:00", "23:59")
    else -> {
        val fmt = SimpleDateFormat("MMM d", Locale.getDefault())
        val mid = rangeStart + (System.currentTimeMillis() - rangeStart) / 2
        listOf(fmt.format(Date(rangeStart)), fmt.format(Date(mid)), "Now")
    }
}

/** Compact chart-axis number — trims ".0", one decimal otherwise. */
private fun fmtChartNumber(v: Double): String {
    val r = (v * 10).toLong() / 10.0
    return if (abs(r - r.toLong()) < 0.0001) r.toLong().toString()
    else "%.1f".format(Locale.getDefault(), r)
}

// =============================================================================
// Filter chip
// =============================================================================

/** Small filter chip — selected: ink fill / white text; idle: elevated/border. */
@Composable
private fun FilterChip(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val shape = RoundedCornerShape(16.dp)
    val bg = if (selected) OhdColors.Ink else OhdColors.BgElevated
    val fg = if (selected) OhdColors.White else OhdColors.Muted
    Box(
        modifier = Modifier
            .height(30.dp)
            .background(bg, shape)
            .border(BorderStroke(1.dp, OhdColors.Line), shape)
            .clickable { onClick() }
            .padding(horizontal = 12.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = if (selected) FontWeight.W500 else FontWeight.W400,
            fontSize = 12.sp,
            color = fg,
        )
    }
}

/** Audit-then-stub fallback used when the structured event stream is empty. */
private fun loadFallbackRows(): List<DisplayRow> {
    val audit = StorageRepository
        .auditQuery(AuditFilter(limit = 50L))
        .getOrNull()
        .orEmpty()
    if (audit.isNotEmpty()) {
        return audit.map { it.toDisplayRow() }
    }
    return StubData.recentEventsSample.map {
        DisplayRow(primary = it.primary, secondary = null, meta = it.meta)
    }
}

/** First-channel correlation_id helper (used for consumption pair matching). */
private fun OhdEvent.correlationId(): String? =
    channels.firstOrNull { it.path == "correlation_id" }
        ?.scalar
        ?.let { (it as? OhdScalar.Text)?.v }

// =============================================================================
// Row composable
// =============================================================================

/**
 * One timeline row: 36 dp circular type-coloured icon, primary + secondary
 * text column, right-side trailing column (relative timestamp + pencil
 * edit affordance).
 *
 * Tappable surface (the whole row body sans the trailing pencil) is wired
 * to [onEdit] today; the pencil click handler does the same — both keep
 * the edit flow one tap away while leaving the body click free for a
 * future "drill into details" page.
 */
@Composable
private fun EventRow(
    event: OhdEvent,
    finishedPair: OhdEvent?,
    nutrition: FoodTotals?,
    onEdit: () -> Unit,
) {
    val visual: EventVisual = visualFor(event.eventType)
    val primary = primaryFor(event, nutrition)
    val secondary = secondaryFor(event, finishedPair)
    val relative = fmtRelative(event.timestampMs)

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable { onEdit() }
            .background(OhdColors.Bg)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // Leading 36 dp tinted circle with the type icon.
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

        // Primary + secondary stack.
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = primary,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 14.sp,
                color = OhdColors.Ink,
            )
            if (secondary != null) {
                Text(
                    text = secondary,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
            }
        }

        // Trailing column: timestamp on top, pencil edit affordance below.
        Column(
            horizontalAlignment = Alignment.End,
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = relative,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
                textAlign = TextAlign.End,
            )
            Box(
                modifier = Modifier
                    .size(28.dp)
                    .clickable { onEdit() },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = OhdIcons.Edit,
                    contentDescription = "Edit",
                    tint = OhdColors.Muted,
                    modifier = Modifier.size(16.dp),
                )
            }
        }
    }
}

// =============================================================================
// Primary / secondary text computation
// =============================================================================

/**
 * Render the primary text of one event: "<Type label> · <key channels>".
 *
 * Picks per-namespace shapes so the row reads as a sentence rather than a
 * dump of channel paths:
 *  - measurement.* → "Glucose · 5.4 mmol/L" / "Blood pressure · 118/76 mmHg".
 *  - medication.taken → "Metformin · 500 mg".
 *  - food.eaten / food.consumption_started → "Oat porridge · 380 kcal · 200 g".
 *  - symptom.* → "Headache · 3/10".
 *  - activity.steps / sleep → "Steps · 8 421" / "Sleep · 7 h 12 m".
 *  - default → "<typeLabel> · <first channel display>".
 */
internal fun primaryFor(event: OhdEvent, nutrition: FoodTotals? = null): String {
    val typeLabel = MetricsRegistry.byEventType(event.eventType)
        ?.description
        ?.substringBefore(" (")
        ?.trim()
        ?: prettyEventType(event.eventType)

    val summary = when {
        event.eventType == "measurement.glucose" -> {
            val v = event.numericChannel("value")
            val unit = event.textChannel("unit") ?: "mmol/L"
            v?.let { "${fmtNumber(it)} $unit" }
        }
        event.eventType == "measurement.blood_pressure" -> {
            val sys = event.numericChannel("systolic_mmhg")
            val dia = event.numericChannel("diastolic_mmhg")
            if (sys != null && dia != null) "${sys.toInt()}/${dia.toInt()} mmHg" else null
        }
        event.eventType == "measurement.weight" -> {
            val kg = event.numericChannel("kg") ?: event.numericChannel("value")
            kg?.let { "${fmtNumber(it)} kg" }
        }
        event.eventType == "measurement.temperature" -> {
            val c = event.numericChannel("celsius") ?: event.numericChannel("value")
            c?.let { "${fmtNumber(it)} °C" }
        }
        event.eventType == "measurement.heart_rate" -> {
            event.numericChannel("bpm")?.let { "${it.toInt()} bpm" }
        }
        event.eventType == "measurement.spo2" -> {
            event.numericChannel("percentage")?.let { "${it.toInt()}%" }
        }
        event.eventType == "measurement.pain" -> {
            val loc = event.textChannel("location")
            val nrs = event.numericChannel("severity_nrs")
            when {
                loc != null && nrs != null -> "$loc · ${fmtNumber(nrs)}/10"
                nrs != null -> "${fmtNumber(nrs)}/10"
                loc != null -> loc
                else -> null
            }
        }
        event.eventType == "medication.taken" -> {
            val name = event.textChannel("med.name")
            val dose = event.numericChannel("med.dose")
            val unit = event.textChannel("med.unit")
            when {
                name != null && dose != null && unit != null ->
                    "$name · ${fmtNumber(dose)} $unit"
                name != null -> name
                else -> null
            }
        }
        event.eventType == "food.eaten" ||
            event.eventType == "food.consumption_started" ||
            event.eventType == "food.consumption_finished" -> {
            val name = event.textChannel("name")
            // Prefer the intake.* children's summed kcal (post-beta28 the
            // parent no longer carries macros); fall back to the legacy
            // `kcal` channel only when no children are linked.
            val kcal = nutrition?.kcal ?: event.numericChannel("kcal")?.toInt()
            val grams = event.numericChannel("actual_grams")
                ?: event.numericChannel("grams")
            buildString {
                if (name != null) append(name)
                if (kcal != null && kcal != 0) {
                    if (isNotEmpty()) append(" · ")
                    append("$kcal kcal")
                }
                if (grams != null) {
                    if (isNotEmpty()) append(" · ")
                    append("${grams.toInt()} g")
                }
                if (nutrition != null) {
                    val macros = buildList {
                        if (nutrition.carbsG > 0) add("${nutrition.carbsG}g C")
                        if (nutrition.proteinG > 0) add("${nutrition.proteinG}g P")
                        if (nutrition.fatG > 0) add("${nutrition.fatG}g F")
                    }
                    if (macros.isNotEmpty()) {
                        if (isNotEmpty()) append(" · ")
                        append(macros.joinToString(" "))
                    }
                }
            }.takeIf { it.isNotEmpty() }
        }
        event.eventType.startsWith("symptom.") -> {
            val sev = event.numericChannel("severity")
            val label = event.textChannel("severity_label")
            when {
                sev != null && label != null -> "$label · ${fmtNumber(sev)}/10"
                sev != null -> "${fmtNumber(sev)}/10"
                label != null -> label
                else -> null
            }
        }
        event.eventType == "activity.steps" -> {
            event.intChannel("count")?.let { fmtThousands(it) + " steps" }
        }
        event.eventType == "activity.sleep" -> {
            event.intChannel("duration_minutes")?.let {
                val h = it / 60
                val m = it % 60
                if (h > 0) "${h}h ${m}m" else "${m}m"
            }
        }
        event.eventType == "emergency.test_run" -> {
            event.textChannel("kind") ?: "test"
        }
        event.eventType == "audit.event_superseded" -> {
            val orig = event.textChannel("original_ulid")?.takeLast(6) ?: "?"
            val new = event.textChannel("new_ulid")?.takeLast(6) ?: "?"
            "$orig → $new"
        }
        else -> null
    }

    val summaryFallback = summary
        ?: event.channels.firstOrNull()?.display
        ?: event.notes
        ?: "(no detail)"
    return "$typeLabel · $summaryFallback"
}

/**
 * Render the secondary (source / context) line of one event.
 *
 * Maps the well-known `source` tags to friendly labels and, for food
 * consumption pairs, appends "(in progress)" / "(finished after …)" so
 * the user can see at a glance whether a sipped beverage closed.
 */
internal fun secondaryFor(event: OhdEvent, finishedPair: OhdEvent?): String? {
    val sourceLabel = when (val s = event.source) {
        null -> null
        "manual:android_app" -> "Manual · this device"
        "health_connect" -> "Health Connect"
        else -> when {
            s.startsWith("healthconnect:") -> "Health Connect"
            s.startsWith("manual:") -> "Manual · ${s.removePrefix("manual:")}"
            else -> s
        }
    }

    val pairNote: String? = when (event.eventType) {
        "food.consumption_started" -> {
            if (finishedPair != null) {
                "finished after ${fmtElapsed(event.timestampMs, finishedPair.timestampMs)}"
            } else {
                "in progress"
            }
        }
        else -> null
    }

    return when {
        sourceLabel != null && pairNote != null -> "$sourceLabel · ($pairNote)"
        sourceLabel != null -> sourceLabel
        pairNote != null -> "($pairNote)"
        else -> null
    }
}

// =============================================================================
// Channel-extraction helpers
// =============================================================================

private fun OhdEvent.numericChannel(path: String): Double? =
    channels.firstOrNull { it.path == path }?.scalar?.let {
        when (it) {
            is OhdScalar.Real -> it.v
            is OhdScalar.Int -> it.v.toDouble()
            else -> null
        }
    }

private fun OhdEvent.intChannel(path: String): Long? =
    channels.firstOrNull { it.path == path }?.scalar?.let {
        when (it) {
            is OhdScalar.Int -> it.v
            is OhdScalar.Real -> it.v.toLong()
            else -> null
        }
    }

private fun OhdEvent.textChannel(path: String): String? =
    channels.firstOrNull { it.path == path }?.scalar?.let {
        when (it) {
            is OhdScalar.Text -> it.v.takeIf { s -> s.isNotBlank() }
            else -> null
        }
    }

/** Format a Double trimming trailing ".0" for whole numbers. */
private fun fmtNumber(v: Double): String {
    val rounded = (v * 10).toLong() / 10.0
    return if (abs(rounded - rounded.toLong()) < 0.0001) rounded.toLong().toString()
    else "%.1f".format(Locale.getDefault(), rounded)
}

/** Insert thousands separators ("8 421"). */
private fun fmtThousands(v: Long): String {
    val s = v.toString()
    val sb = StringBuilder()
    for ((i, c) in s.withIndex()) {
        if (i > 0 && (s.length - i) % 3 == 0) sb.append(' ')
        sb.append(c)
    }
    return sb.toString()
}

// =============================================================================
// Legacy DisplayRow / audit fallback
// =============================================================================

/**
 * One screen-row shape — three optional strings forming
 * `<EventType> · <human summary>` / `<secondary?>` / `<timestamp>`.
 *
 * Retained for the audit / stub fallback path, where the timeline row
 * shape isn't available (no [OhdEvent] to derive an icon from).
 */
internal data class DisplayRow(
    val primary: String,
    val secondary: String?,
    val meta: String,
)

/**
 * Convert an [AuditEntry] into a display row. Used when there are no
 * events yet but the audit log has entries (e.g. brand-new install with
 * only the self-session-token grant_mgmt rows).
 */
internal fun AuditEntry.toDisplayRow(): DisplayRow = DisplayRow(
    primary = "${opKind.replaceFirstChar { it.uppercase() }} · $opName",
    secondary = querySummary,
    meta = fmtRecentTimestamp(tsMs),
)

/**
 * "Today HH:mm" / "Yesterday HH:mm" / "yyyy-MM-dd HH:mm" — the format the
 * Pencil PNG shows for the right-meta field. Locale-aware via
 * [SimpleDateFormat]; the calendar comparison is for *the device's* day
 * boundary so timezone-shifted entries land in the right bucket.
 */
internal fun fmtRecentTimestamp(ms: Long, now: Long = System.currentTimeMillis()): String {
    val time = SimpleDateFormat("HH:mm", Locale.getDefault()).format(Date(ms))
    val cal = Calendar.getInstance().apply { timeInMillis = ms }
    val today = Calendar.getInstance().apply { timeInMillis = now }
    val yesterday = (today.clone() as Calendar).apply { add(Calendar.DAY_OF_YEAR, -1) }

    fun sameDay(a: Calendar, b: Calendar): Boolean =
        a.get(Calendar.YEAR) == b.get(Calendar.YEAR) &&
            a.get(Calendar.DAY_OF_YEAR) == b.get(Calendar.DAY_OF_YEAR)

    return when {
        sameDay(cal, today) -> "Today $time"
        sameDay(cal, yesterday) -> "Yesterday $time"
        else -> SimpleDateFormat("yyyy-MM-dd HH:mm", Locale.getDefault()).format(Date(ms))
    }
}

// =============================================================================
// Bridge helpers used by EditEventScreen lookup path. Kept here so the
// "find by ulid" client-side filter has a single owner.
// =============================================================================

/**
 * Pull `intake.*` children covering the time span of the loaded food events
 * and group them by `correlation_id`, summing `value:real` per event_type.
 *
 * One query, then a single client-side aggregation — N+1 avoided. The
 * result keys are correlation_id strings; lookups by `event.correlationId()`
 * surface the macros computed from the children of a given parent.
 */
private fun aggregateFoodNutrition(events: List<OhdEvent>): Map<String, FoodTotals> {
    val foodEvents = events.filter {
        it.eventType == "food.eaten" ||
            it.eventType == "food.consumption_started" ||
            it.eventType == "food.consumption_finished"
    }
    if (foodEvents.isEmpty()) return emptyMap()
    val correlationIds = foodEvents.mapNotNull { it.correlationId() }.toSet()
    if (correlationIds.isEmpty()) return emptyMap()
    // Time window: 1 hour before earliest food row → 1 hour after latest.
    // Intake children are emitted with the parent's timestamp so they live
    // in the same band; the buffer absorbs gradual-consumption pairs whose
    // `started` and child timestamps can drift across a few minutes.
    val fromMs = foodEvents.minOf { it.timestampMs } - 60 * 60 * 1_000L
    val toMs = foodEvents.maxOf { it.timestampMs } + 60 * 60 * 1_000L
    val intake = StorageRepository.queryEvents(
        EventFilter(
            fromMs = fromMs,
            toMs = toMs,
            eventTypesIn = INTAKE_EVENT_TYPES,
            visibility = EventVisibility.All,
            limit = 5_000,
        ),
    ).getOrNull().orEmpty()
    val byCorrelation: Map<String, List<OhdEvent>> = intake.groupBy { ev ->
        ev.channels.firstOrNull { it.path == "correlation_id" }
            ?.let { (it.scalar as? OhdScalar.Text)?.v }
            .orEmpty()
    }.filterKeys { it.isNotEmpty() && it in correlationIds }
    return byCorrelation.mapValues { (_, children) -> aggregateIntakeChildren(children) }
}

/**
 * Find a single event by ULID. Pulls the most recent 200 events (no
 * `eventUlidsIn` is exposed via uniffi — see comment on
 * `EventFilterDto.into_core`) and scans client-side. Returns `null` when
 * the event isn't in the recent window or storage isn't open.
 */
internal fun findEventByUlid(ulid: String): OhdEvent? {
    val events = StorageRepository
        .queryEvents(com.ohd.connect.data.EventFilter(limit = 200L))
        .getOrNull()
        .orEmpty()
    return events.firstOrNull { it.ulid == ulid }
}
