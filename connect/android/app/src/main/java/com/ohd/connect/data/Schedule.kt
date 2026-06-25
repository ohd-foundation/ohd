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

    /** Next firing strictly after `afterMs`, or null if none within ~1 year. */
    fun nextAfter(afterMs: Long): Long? {
        if (this is Unscheduled) return null
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
        if (this is Unscheduled) return null
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
        if (this is Unscheduled) return DueStatus.Unscheduled
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
            if (s.startsWith("anchor:")) {
                val name = s.removePrefix("anchor:").trim().lowercase()
                val times = ANCHORS[name] ?: return Unscheduled
                return Anchor(name, times)
            }
            return parseCron(s) ?: Unscheduled
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
