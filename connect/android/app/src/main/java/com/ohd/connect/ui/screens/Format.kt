package com.ohd.connect.ui.screens

import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.roundToInt

/** Misc small format helpers shared across screens. */

internal val DateFormatter = SimpleDateFormat("yyyy-MM-dd HH:mm", Locale.getDefault())

internal fun fmtDate(ms: Long): String = DateFormatter.format(Date(ms))

/** "3m ago", "2h ago", "1d ago", or fall back to absolute date. */
internal fun fmtRelative(ms: Long, now: Long = System.currentTimeMillis()): String {
    val dt = now - ms
    if (dt < 0) return fmtDate(ms)
    val sec = (dt / 1000.0).roundToInt()
    if (sec < 60) return "${sec}s ago"
    val min = (sec / 60.0).roundToInt()
    if (min < 60) return "${min}m ago"
    val hr = (min / 60.0).roundToInt()
    if (hr < 24) return "${hr}h ago"
    val day = (hr / 24.0).roundToInt()
    if (day < 7) return "${day}d ago"
    return fmtDate(ms)
}

internal fun fmtElapsed(fromMs: Long, toMs: Long = System.currentTimeMillis()): String {
    val sec = ((toMs - fromMs) / 1000).coerceAtLeast(0)
    if (sec < 60) return "${sec}s"
    val min = sec / 60
    if (min < 60) return "${min}m"
    val hr = min / 60
    if (hr < 24) return "${hr}h ${min % 60}m"
    val days = hr / 24
    return "${days}d ${hr % 24}h"
}

internal fun prettyEventType(t: String): String = when (t) {
    "std.blood_glucose" -> "Glucose"
    "std.heart_rate_resting" -> "Heart rate"
    "std.body_temperature" -> "Temperature"
    "std.blood_pressure" -> "Blood pressure"
    "std.medication_dose" -> "Medication"
    "std.symptom" -> "Symptom"
    "std.meal" -> "Meal"
    "std.mood" -> "Mood"
    "std.clinical_note" -> "Clinical note"
    else -> t.removePrefix("std.")
}
