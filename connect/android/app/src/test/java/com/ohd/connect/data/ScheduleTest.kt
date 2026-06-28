package com.ohd.connect.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Calendar

/**
 * Unit tests for the schedule evaluator. Timestamps are built with the same
 * default-timezone `Calendar` the evaluator uses, so assertions are
 * tz-independent.
 */
class ScheduleTest {

    private fun at(y: Int, month1: Int, d: Int, h: Int, min: Int): Long =
        Calendar.getInstance().apply {
            set(y, month1 - 1, d, h, min, 0)
            set(Calendar.MILLISECOND, 0)
        }.timeInMillis

    @Test fun parses_daily_cron() {
        val s = Schedule.parse("0 8 * * *")
        assertTrue(s is Schedule.Cron)
        s as Schedule.Cron
        assertEquals(setOf(0), s.minute)
        assertEquals(setOf(8), s.hour)
        assertNull(s.dayOfMonth)
        assertNull(s.dayOfWeek)
    }

    @Test fun parses_anchor() {
        val s = Schedule.parse("anchor:lunch")
        assertTrue(s is Schedule.Anchor)
        assertEquals(listOf(12 to 30), (s as Schedule.Anchor).times)
    }

    @Test fun unparseable_and_as_needed_are_unscheduled() {
        assertTrue(Schedule.parse("") is Schedule.Unscheduled)
        assertTrue(Schedule.parse("whenever i remember") is Schedule.Unscheduled)
        assertTrue(Schedule.parse("anchor:as_needed") is Schedule.Unscheduled)
        assertTrue(Schedule.parse(null) is Schedule.Unscheduled)
    }

    @Test fun daily_next_and_last() {
        val s = Schedule.parse("0 8 * * *")
        val now = at(2026, 6, 15, 9, 0)
        assertEquals(at(2026, 6, 16, 8, 0), s.nextAfter(now))
        assertEquals(at(2026, 6, 15, 8, 0), s.lastBefore(now))
    }

    @Test fun every_8h_next_and_last() {
        val s = Schedule.parse("0 */8 * * *") // 00:00, 08:00, 16:00
        val now = at(2026, 6, 15, 9, 0)
        assertEquals(at(2026, 6, 15, 16, 0), s.nextAfter(now))
        assertEquals(at(2026, 6, 15, 8, 0), s.lastBefore(now))
    }

    @Test fun due_now_within_grace() {
        val s = Schedule.parse("0 8 * * *")
        val status = s.dueStatus(lastLogMs = null, now = at(2026, 6, 15, 9, 0))
        assertTrue("expected DueNow, got $status", status is DueStatus.DueNow)
    }

    @Test fun overdue_past_grace() {
        val s = Schedule.parse("0 8 * * *")
        val status = s.dueStatus(lastLogMs = null, now = at(2026, 6, 15, 11, 30))
        assertTrue("expected Overdue, got $status", status is DueStatus.Overdue)
        assertEquals(at(2026, 6, 15, 8, 0), (status as DueStatus.Overdue).sinceMs)
    }

    @Test fun taken_for_slot_when_logged_after_slot() {
        val s = Schedule.parse("0 8 * * *")
        val status = s.dueStatus(
            lastLogMs = at(2026, 6, 15, 8, 30),
            now = at(2026, 6, 15, 9, 0),
        )
        assertTrue("expected Taken, got $status", status is DueStatus.Taken)
        assertEquals(at(2026, 6, 16, 8, 0), (status as DueStatus.Taken).nextMs)
    }

    @Test fun parses_interval_forms() {
        val day = 24L * 60 * 60 * 1000
        assertEquals(Schedule.Interval(7 * day), Schedule.parse("weekly"))
        assertEquals(Schedule.Interval(7 * day), Schedule.parse("every 7 days"))
        assertEquals(Schedule.Interval(7 * day), Schedule.parse("every:7d"))
        assertEquals(Schedule.Interval(8 * 60 * 60 * 1000), Schedule.parse("every 8h"))
        // count-per-day ("twice daily", "daily") are fixed clock times now,
        // not floating intervals.
        assertEquals(setOf(9, 21), (Schedule.parse("twice daily") as Schedule.Cron).hour)
        assertEquals(setOf(9), (Schedule.parse("daily") as Schedule.Cron).hour)
    }

    @Test fun interval_due_overdue_taken() {
        val day = 24L * 60 * 60 * 1000
        val s = Schedule.parse("weekly")
        val now = at(2026, 6, 15, 9, 0)
        // never logged → take the first now
        assertTrue(s.dueStatus(null, now) is DueStatus.DueNow)
        // last dose 8 days ago → overdue (next was a day ago)
        assertTrue(s.dueStatus(now - 8 * day, now) is DueStatus.Overdue)
        // last dose 3 days ago → satisfied, next in 4 days
        val taken = s.dueStatus(now - 3 * day, now)
        assertTrue(taken is DueStatus.Taken)
        assertEquals(now - 3 * day + 7 * day, (taken as DueStatus.Taken).nextMs)
    }

    @Test fun interval_natural_morning_maps_to_anchor() {
        assertTrue(Schedule.parse("every morning") is Schedule.Anchor)
    }

    @Test fun parses_sig_count_codes_to_fixed_times() {
        assertEquals(setOf(9, 21), (Schedule.parse("BID") as Schedule.Cron).hour)
        assertEquals(setOf(8, 14, 22), (Schedule.parse("tid") as Schedule.Cron).hour)
        assertEquals(setOf(8, 12, 16, 20), (Schedule.parse("QID") as Schedule.Cron).hour)
        assertEquals(setOf(9), (Schedule.parse("once daily") as Schedule.Cron).hour)
    }

    @Test fun parses_times_dsl_and_starting_at() {
        assertEquals(setOf(6, 14, 22), (Schedule.parse("times:3@6,14,22") as Schedule.Cron).hour)
        assertEquals(setOf(6, 14, 22), (Schedule.parse("every 8h starting at 6") as Schedule.Cron).hour)
    }

    @Test fun parses_meal_relative() {
        assertEquals(Schedule.Meal(Schedule.Meal.Relation.With, false), Schedule.parse("with food"))
        assertEquals(Schedule.Meal(Schedule.Meal.Relation.Before, false), Schedule.parse("ac"))
        assertEquals(Schedule.Meal(Schedule.Meal.Relation.After, false), Schedule.parse("after meals"))
        assertEquals(Schedule.Meal(Schedule.Meal.Relation.With, true), Schedule.parse("with breakfast"))
        assertEquals(Schedule.Meal(Schedule.Meal.Relation.With, true), Schedule.parse("meal:with:first"))
    }

    @Test fun parses_event_and_prn() {
        assertEquals(Schedule.Event("waking"), Schedule.parse("first thing in the morning"))
        assertEquals(Schedule.Event("waking"), Schedule.parse("event:waking"))
        assertTrue(Schedule.parse("as needed") is Schedule.Prn)
        assertTrue(Schedule.parse("prn") is Schedule.Prn)
    }

    @Test fun parses_conditional() {
        assertEquals(Schedule.Conditional("glucose", ">", 14.0), Schedule.parse("take when glucose over 14"))
        assertEquals(Schedule.Conditional("glucose", ">", 14.0), Schedule.parse("cond:glucose>14"))
        assertEquals(Schedule.Conditional("temperature", ">", 38.0), Schedule.parse("if temperature above 38"))
    }

    @Test fun event_driven_kinds_have_no_clock_due() {
        val now = at(2026, 6, 15, 9, 0)
        assertTrue(Schedule.parse("with food").dueStatus(null, now) is DueStatus.Unscheduled)
        assertTrue(Schedule.parse("prn").dueStatus(null, now) is DueStatus.Unscheduled)
        assertTrue(Schedule.parse("cond:glucose>14").dueStatus(null, now) is DueStatus.Unscheduled)
    }

    @Test fun unscheduled_status() {
        assertTrue(Schedule.Unscheduled.dueStatus(null, at(2026, 6, 15, 9, 0)) is DueStatus.Unscheduled)
    }

    @Test fun weekly_dow_matches_only_those_days() {
        // Mon (cron 1) & Thu (cron 4) at 08:00. 2026-06-15 is a Monday.
        val s = Schedule.parse("0 8 * * 1,4")
        val monNext = s.nextAfter(at(2026, 6, 15, 9, 0)) // Mon 09:00 → next is Thu 08:00
        assertEquals(at(2026, 6, 18, 8, 0), monNext)
    }
}
