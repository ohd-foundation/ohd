package com.ohd.connect.data

import java.util.Calendar

/**
 * Evaluation layer over the loose `schedule` string stored on medication
 * regimens and measurement watches (plan deep-dancing-teacup.md). The string
 * is one of:
 *
 *  - a 5-field cron expression — `min hour day-of-month month day-of-week`,
 *    e.g. `0 8 * * *` (daily 08:00), `0 0,8,16 * * *` (every 8h), `0 8 * * 1,4`
 *    (Mon & Thu 08:00). Fields support a wildcard, steps, ranges (`a-b`) and
 *    lists (`a,b`).
 *  - `anchor:<name>` — a contextual time-of-day: `waking`, `first_food`,
 *    `breakfast`, `lunch`, `dinner`, `bedtime`, `each_meal`, `as_needed`.
 *    v1 resolves anchors to nominal local clock times (firing relative to a
 *    logged meal event is a future refinement).
 *  - anything else / empty → [Unscheduled].
 *
 * Pure + timezone-aware via the device's default `Calendar`. No persistence,
 * no engine state — callers pass `now` and the item's last-log timestamp.
 */
sealed interface Schedule {

    /** Does this schedule fire at the (minute-resolution) instant in `cal`? */
    fun matchesMinute(cal: Calendar): Boolean

    /** No evaluable schedule — `as_needed`, free text, blank, unparseable. */
    object Unscheduled : Schedule {
        override fun matchesMinute(cal: Calendar): Boolean = false
    }

    /** A 5-field cron expression. `null` field = `*` (matches anything). */
    data class Cron(
        val minute: Set<Int>?,
        val hour: Set<Int>?,
        val dayOfMonth: Set<Int>?,
        val month: Set<Int>?,
        val dayOfWeek: Set<Int>?,
    ) : Schedule {
        override fun matchesMinute(cal: Calendar): Boolean {
            fun ok(set: Set<Int>?, v: Int) = set == null || v in set
            val dowCron = cal.get(Calendar.DAY_OF_WEEK) - 1 // Calendar 1=Sun → cron 0=Sun
            val dowOk = dayOfWeek == null || dowCron in dayOfWeek ||
                (dowCron == 0 && 7 in dayOfWeek) // cron allows 7 for Sunday
            return ok(minute, cal.get(Calendar.MINUTE)) &&
                ok(hour, cal.get(Calendar.HOUR_OF_DAY)) &&
                ok(dayOfMonth, cal.get(Calendar.DAY_OF_MONTH)) &&
                ok(month, cal.get(Calendar.MONTH) + 1) &&
                dowOk
        }
    }

    /** One or more nominal times-of-day (hour, minute). */
    data class Anchor(val name: String, val times: List<Pair<Int, Int>>) : Schedule {
        override fun matchesMinute(cal: Calendar): Boolean =
            times.any { (h, m) -> cal.get(Calendar.HOUR_OF_DAY) == h && cal.get(Calendar.MINUTE) == m }
    }

    /**
     * A floating interval relative to the last log — "every 7 days from when
     * you took it", a weekly injection. Not clock-anchored, so it's evaluated
     * directly off `lastLogMs` rather than via [nextAfter]/[lastBefore].
     */
    data class Interval(val stepMs: Long) : Schedule {
        override fun matchesMinute(cal: Calendar): Boolean = false
    }

    /** PRN — take as needed; explicit "no schedule, never nag". */
    object Prn : Schedule {
        override fun matchesMinute(cal: Calendar): Boolean = false
    }

    /**
     * Meal-relative (AC/PC/CC) — fires off `food.eaten` events, handled by the
     * trigger engine, not the clock. [firstOfDay] = only the day's first meal
     * ("with breakfast" / "with first food").
     */
    data class Meal(val relation: Relation, val firstOfDay: Boolean) : Schedule {
        enum class Relation { Before, With, After }
        override fun matchesMinute(cal: Calendar): Boolean = false
    }

    /**
     * Event-anchored — `waking` (off `activity.sleep` end) or `bedtime`
     * (sleep start). Handled by the trigger engine.
     */
    data class Event(val name: String) : Schedule {
        override fun matchesMinute(cal: Calendar): Boolean = false
    }

    /**
     * Conditional / sliding-scale — fire when a `measurement.<metric>` reading
     * crosses [value] per [op] (`>`, `<`, `>=`, `<=`). Notify-only; never
     * auto-doses. Handled by the trigger engine.
     */
    data class Conditional(val metric: String, val op: String, val value: Double) : Schedule {
        override fun matchesMinute(cal: Calendar): Boolean = false
    }

    /** Next firing strictly after `afterMs`, or null if none within ~1 year. */
    fun nextAfter(afterMs: Long): Long? {
        // Only clock-anchored kinds have minute-resolvable slots.
        if (this !is Cron && this !is Anchor) return null
        val cal = Calendar.getInstance().apply {
            timeInMillis = afterMs
            set(Calendar.SECOND, 0)
            set(Calendar.MILLISECOND, 0)
            add(Calendar.MINUTE, 1)
        }
        repeat(SEARCH_MINUTES) {
            if (matchesMinute(cal)) return cal.timeInMillis
            cal.add(Calendar.MINUTE, 1)
        }
        return null
    }

    /** Most recent firing at or before `beforeMs`, or null if none in ~1 year. */
    fun lastBefore(beforeMs: Long): Long? {
        // Only clock-anchored kinds have minute-resolvable slots.
        if (this !is Cron && this !is Anchor) return null
        val cal = Calendar.getInstance().apply {
            timeInMillis = beforeMs
            set(Calendar.SECOND, 0)
            set(Calendar.MILLISECOND, 0)
        }
        repeat(SEARCH_MINUTES) {
            if (matchesMinute(cal)) return cal.timeInMillis
            cal.add(Calendar.MINUTE, -1)
        }
        return null
    }

    /**
     * Where this item stands relative to its schedule, given the timestamp of
     * its most recent log (a dose / a reading), or null if never logged.
     */
    fun dueStatus(lastLogMs: Long?, now: Long): DueStatus {
        // Event-driven kinds have no clock due-state; the trigger engine drives
        // them. PRN/unscheduled never nag.
        if (this is Unscheduled || this is Prn || this is Meal ||
            this is Event || this is Conditional
        ) {
            return DueStatus.Unscheduled
        }
        if (this is Interval) {
            // Floating: the next dose is `stepMs` after the last one. Never
            // logged → the first is due now.
            if (lastLogMs == null) return DueStatus.DueNow
            val next = lastLogMs + stepMs
            if (now < next) return DueStatus.Taken(next)
            val overdueBy = now - next
            return if (overdueBy <= DUE_GRACE_MS) DueStatus.DueNow else DueStatus.Overdue(next)
        }
        val prev = lastBefore(now)
        val next = nextAfter(now)
        if (prev == null) {
            return if (next != null) DueStatus.Upcoming(next) else DueStatus.Unscheduled
        }
        // The most recent slot is satisfied if a log landed at or after it.
        if (lastLogMs != null && lastLogMs >= prev) {
            return DueStatus.Taken(next)
        }
        val overdueBy = now - prev
        return if (overdueBy <= DUE_GRACE_MS) DueStatus.DueNow else DueStatus.Overdue(prev)
    }

    /**
     * Timestamp of the slot that is currently due/overdue and unsatisfied, or
     * null when nothing is pending. Used by the reminder worker to dedup one
     * nudge per slot across all schedule kinds.
     */
    fun currentSlotMs(lastLogMs: Long?, now: Long): Long? = when (this) {
        is Unscheduled -> null
        is Interval ->
            // Don't background-nag a never-started interval (the screen still
            // shows "due now"); once logged, the slot is last + step.
            lastLogMs?.let { it + stepMs }?.takeIf { now >= it }
        else -> when (dueStatus(lastLogMs, now)) {
            is DueStatus.DueNow, is DueStatus.Overdue -> lastBefore(now)
            else -> null
        }
    }

    companion object {
        /** ~1 year of minutes — the bounded search window for next/last. */
        private const val SEARCH_MINUTES = 366 * 24 * 60

        /** A slot is "due now" until this long past it, then "overdue". */
        const val DUE_GRACE_MS = 2L * 60L * 60L * 1000L

        /** Nominal local times for each anchor name. */
        private val ANCHORS: Map<String, List<Pair<Int, Int>>> = mapOf(
            "waking" to listOf(7 to 0),
            "first_food" to listOf(8 to 0),
            "breakfast" to listOf(8 to 0),
            "lunch" to listOf(12 to 30),
            "dinner" to listOf(18 to 30),
            "bedtime" to listOf(22 to 0),
            "each_meal" to listOf(8 to 0, 12 to 30, 18 to 30),
            // as_needed → no schedule (handled below).
        )

        fun parse(raw: String?): Schedule {
            val s = raw?.trim().orEmpty()
            if (s.isEmpty()) return Unscheduled
            val lower = s.lowercase()
            // Explicit DSL prefixes (what the UI builder + LLM emit).
            when {
                lower.startsWith("anchor:") -> {
                    val name = lower.removePrefix("anchor:").trim()
                    return ANCHORS[name]?.let { Anchor(name, it) } ?: Unscheduled
                }
                lower.startsWith("every:") ->
                    return parseInterval(lower.removePrefix("every:").trim()) ?: Unscheduled
                lower.startsWith("times:") ->
                    return parseTimes(lower.removePrefix("times:").trim()) ?: Unscheduled
                lower.startsWith("meal:") ->
                    return parseMealSpec(lower.removePrefix("meal:").trim()) ?: Unscheduled
                lower.startsWith("event:") ->
                    return parseEventSpec(lower.removePrefix("event:").trim()) ?: Unscheduled
                lower.startsWith("cond:") ->
                    return parseCondSpec(lower.removePrefix("cond:").trim()) ?: Unscheduled
                lower == "prn" -> return Prn
            }
            // A literal 5-field cron expression.
            parseCron(s)?.let { return it }
            // Otherwise sig codes / plain English.
            return parseNatural(lower) ?: Unscheduled
        }

        // ---- DSL spec parsers ------------------------------------------------

        /** `3@6,14,22` / `2@9,21` / `2` (default spacing) → fixed clock slots. */
        private fun parseTimes(spec: String): Schedule? {
            val parts = spec.split("@")
            val n = parts[0].trim().toIntOrNull() ?: return null
            if (n <= 0) return null
            val hours = if (parts.size > 1) {
                parts[1].split(",").mapNotNull { it.trim().toIntOrNull() }.filter { it in 0..23 }
            } else {
                defaultTimesFor(n)
            }
            if (hours.isEmpty()) return null
            return Cron(minute = setOf(0), hour = hours.toSortedSet(), dayOfMonth = null, month = null, dayOfWeek = null)
        }

        /** Reasonable clock times for an N-times-a-day count with no times given. */
        private fun defaultTimesFor(n: Int): List<Int> = when (n) {
            1 -> listOf(9)
            2 -> listOf(9, 21)
            3 -> listOf(8, 14, 22)
            4 -> listOf(8, 12, 16, 20)
            else -> (0 until n).map { 8 + it * 24 / n }.filter { it in 0..23 }
        }

        /** `with` / `before` / `after`, optional `:first`. */
        private fun parseMealSpec(spec: String): Meal? {
            val first = spec.endsWith(":first")
            val rel = when (spec.removeSuffix(":first").trim()) {
                "before" -> Meal.Relation.Before
                "with" -> Meal.Relation.With
                "after" -> Meal.Relation.After
                else -> return null
            }
            return Meal(rel, first)
        }

        private fun parseEventSpec(spec: String): Schedule? = when (spec.trim()) {
            "waking", "bedtime" -> Event(spec.trim())
            else -> null
        }

        /** `glucose>14` / `bp>=140` → a [Conditional]. */
        private fun parseCondSpec(spec: String): Conditional? {
            val m = Regex("^([a-z_]+)(>=|<=|>|<)(\\d+(?:\\.\\d+)?)$")
                .find(spec.replace(" ", "")) ?: return null
            val v = m.groupValues[3].toDoubleOrNull() ?: return null
            return Conditional(m.groupValues[1], m.groupValues[2], v)
        }

        private val DAY_MS = 24L * 60L * 60L * 1000L
        private val HOUR_MS = 60L * 60L * 1000L

        /** `7d` / `12h` / `30m` / `2w` → an [Interval], or null. */
        private fun parseInterval(token: String): Interval? {
            val m = Regex("^(\\d+)\\s*([dhmw])$").find(token.replace(" ", "")) ?: return null
            val n = m.groupValues[1].toLongOrNull() ?: return null
            if (n <= 0) return null
            val step = when (m.groupValues[2]) {
                "d" -> n * DAY_MS
                "h" -> n * HOUR_MS
                "m" -> n * 60L * 1000L
                "w" -> n * 7L * DAY_MS
                else -> return null
            }
            return Interval(step)
        }

        /**
         * Sig codes (BID/TID/QID/AC/PC/HS/PRN…) + plain English → a schedule.
         * Best-effort; null if nothing matches. Order matters: specific
         * patterns (conditional, "starting at", explicit intervals) are tried
         * before the keyword table.
         */
        private fun parseNatural(raw: String): Schedule? {
            val s = raw.replace(".", "") // b.i.d → bid
            parseConditionalNatural(s)?.let { return it }
            // "every N h starting at H" → fixed slots H, H+N, …
            Regex("every\\s+(\\d+)\\s*h(?:ours?)?\\s+(?:starting(?:\\s+at)?|from|at)\\s+(\\d{1,2})")
                .find(s)?.let { m ->
                    val step = m.groupValues[1].toInt()
                    val start = m.groupValues[2].toInt()
                    if (step in 1..24 && start in 0..23) {
                        val hours = generateSequence(start) { it + step }.takeWhile { it < 24 }.toList()
                        return Cron(setOf(0), hours.toSortedSet(), null, null, null)
                    }
                }
            // floating "every N day(s)/week(s)/hour(s)"
            Regex("every\\s+(\\d+)\\s*(day|days|d|week|weeks|w|hour|hours|h)").find(s)?.let { m ->
                return parseInterval(m.groupValues[1] + m.groupValues[2].first())
            }
            return when {
                "as needed" in s || "as required" in s || "when needed" in s || "if needed" in s ||
                    matchesWord(s, "prn") || matchesWord(s, "sos") -> Prn

                matchesWord(s, "qid") || "four times" in s || "4 times" in s -> parseTimes("4")
                matchesWord(s, "tid") || "three times" in s || "3 times" in s -> parseTimes("3")
                matchesWord(s, "bid") || matchesWord(s, "bd") || "twice" in s -> parseTimes("2")

                matchesWord(s, "ac") || "before meal" in s || "before food" in s || "before eating" in s ->
                    Meal(Meal.Relation.Before, false)
                matchesWord(s, "pc") || "after meal" in s || "after food" in s || "after eating" in s ->
                    Meal(Meal.Relation.After, false)
                "with breakfast" in s || "with first food" in s -> Meal(Meal.Relation.With, true)
                matchesWord(s, "cc") || "with food" in s || "with meal" in s -> Meal(Meal.Relation.With, false)

                "first thing" in s || "on waking" in s || "upon waking" in s ||
                    "when i wake" in s || "after waking" in s -> Event("waking")

                matchesWord(s, "hs") || matchesWord(s, "qhs") || "bedtime" in s ||
                    "at night" in s || "before bed" in s -> Anchor("bedtime", ANCHORS.getValue("bedtime"))
                matchesWord(s, "qam") || "every morning" in s || "morning" in s || matchesWord(s, "mane") ->
                    Anchor("breakfast", ANCHORS.getValue("breakfast"))
                matchesWord(s, "qpm") || "every evening" in s || "evening" in s ->
                    Anchor("dinner", ANCHORS.getValue("dinner"))
                "lunch" in s || "noon" in s -> Anchor("lunch", ANCHORS.getValue("lunch"))

                "every other day" in s -> Interval(2 * DAY_MS)
                "weekly" in s || "every week" in s || "once a week" in s -> Interval(7 * DAY_MS)
                "fortnight" in s || "biweekly" in s -> Interval(14 * DAY_MS)
                "monthly" in s || "every month" in s -> Interval(30 * DAY_MS)

                "daily" in s || "every day" in s || "once a day" in s || "once daily" in s ||
                    matchesWord(s, "qd") || matchesWord(s, "od") -> parseTimes("1")
                else -> null
            }
        }

        private fun matchesWord(s: String, word: String): Boolean =
            Regex("\\b" + Regex.escape(word) + "\\b").containsMatchIn(s)

        /** "when/if <metric> over/above/under/below <value>" → [Conditional]. */
        private fun parseConditionalNatural(s: String): Conditional? {
            val m = Regex(
                "([a-z ]+?)\\s+(over|above|greater than|under|below|less than)\\s+(\\d+(?:\\.\\d+)?)",
            ).find(s) ?: return null
            val metric = normalizeMetric(m.groupValues[1].trim()) ?: return null
            val op = when (m.groupValues[2].trim()) {
                "over", "above", "greater than" -> ">"
                else -> "<"
            }
            val v = m.groupValues[3].toDoubleOrNull() ?: return null
            return Conditional(metric, op, v)
        }

        /** Map a spoken metric name to the canonical token, or null. */
        private fun normalizeMetric(name: String): String? = when {
            "glucose" in name || "sugar" in name -> "glucose"
            "blood pressure" in name || name.trim() == "bp" -> "blood_pressure"
            "temperature" in name || "temp" in name || "fever" in name -> "temperature"
            "weight" in name -> "weight"
            "heart rate" in name || "pulse" in name || name.trim() == "hr" -> "heart_rate"
            "spo2" in name || "oxygen" in name || "saturation" in name -> "spo2"
            else -> null
        }

        private fun parseCron(s: String): Cron? {
            val parts = s.split(Regex("\\s+"))
            if (parts.size != 5) return null
            return try {
                Cron(
                    minute = parseField(parts[0], 0, 59),
                    hour = parseField(parts[1], 0, 23),
                    dayOfMonth = parseField(parts[2], 1, 31),
                    month = parseField(parts[3], 1, 12),
                    dayOfWeek = parseField(parts[4], 0, 7),
                )
            } catch (_: NumberFormatException) {
                null
            }
        }

        /** One cron field → the set of allowed values, or null for `*`. */
        private fun parseField(token: String, min: Int, max: Int): Set<Int>? {
            if (token == "*") return null
            val out = sortedSetOf<Int>()
            for (part in token.split(",")) {
                when {
                    part.startsWith("*/") -> {
                        val step = part.removePrefix("*/").toInt()
                        if (step <= 0) throw NumberFormatException("step")
                        var v = min
                        while (v <= max) {
                            out.add(v); v += step
                        }
                    }
                    "-" in part -> {
                        val (a, b) = part.split("-").map { it.toInt() }
                        for (v in a..b) if (v in min..max) out.add(v)
                    }
                    else -> {
                        val v = part.toInt()
                        if (v in min..max) out.add(v)
                    }
                }
            }
            if (out.isEmpty()) throw NumberFormatException("empty field")
            return out
        }
    }
}

/** The position of a tracked item relative to its schedule. */
sealed interface DueStatus {
    /** No evaluable cadence. */
    object Unscheduled : DueStatus
    /** Not yet due; next firing at [nextMs]. */
    data class Upcoming(val nextMs: Long) : DueStatus
    /** Within the grace window of its slot — do it now. */
    object DueNow : DueStatus
    /** Past its slot with nothing logged since [sinceMs]. */
    data class Overdue(val sinceMs: Long) : DueStatus
    /** The current slot is satisfied; [nextMs] is the following firing (or null). */
    data class Taken(val nextMs: Long?) : DueStatus
}
