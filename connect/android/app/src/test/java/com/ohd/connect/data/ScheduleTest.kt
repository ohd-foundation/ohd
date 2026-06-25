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
        assertTrue(Schedule.parse("every day-ish") is Schedule.Unscheduled)
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
