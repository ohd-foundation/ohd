package com.ohd.emergency.ui.screens

import androidx.compose.foundation.Canvas
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.filled.LocalHospital
import androidx.compose.material.icons.filled.MedicalServices
import androidx.compose.material.icons.filled.MonitorHeart
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

import com.ohd.emergency.data.CaseVault
import com.ohd.emergency.data.EmergencyRepository
import com.ohd.emergency.data.MedicationEntry
import com.ohd.emergency.data.OperatorSession
import com.ohd.emergency.data.PatientView
import com.ohd.emergency.data.VitalReading
import com.ohd.emergency.data.VitalSnapshot
import com.ohd.emergency.ui.components.ChipTone
import com.ohd.emergency.ui.components.CriticalCard
import com.ohd.emergency.ui.components.EmergencyTopBar
import com.ohd.emergency.ui.components.StatusChip
import com.ohd.emergency.ui.components.SyncIndicatorState
import com.ohd.emergency.ui.theme.EmergencyPalette
import com.ohd.emergency.ui.theme.MonoStyle
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.max
import kotlin.math.min

/**
 * Patient view — the most-looked-at screen during a case.
 *
 * Layout (top → bottom):
 *   - Top bar (operator, sync, panic logout, active-case dot)
 *   - Header card: patient label, age/sex, case ULID, time elapsed, auto-grant chip
 *   - **Critical info card** (red border): allergies, blood type, advance directives
 *   - Active medications list
 *   - Recent vitals row (5 cards with sparklines)
 *   - Active diagnoses (chip cloud)
 *   - Recent observations (timeline-ette)
 *   - Bottom nav bar: Patient (here), Intervention, Timeline, Handoff
 *
 * Per the brief: "Critical info above the fold (red-bordered card):
 * allergies, blood type, advance directives, current diagnoses." We
 * separate diagnoses out of the red card so the card stays focused on
 * "things that change minute-to-minute treatment" — diagnoses live below
 * because they're context, not branch-decision data.
 *
 * "Hide non-emergency data" toggle: per the patient's emergency profile
 * sensitivity classes. Default off (all classes the profile permits are
 * shown). Tapping it filters out anything tagged `mental_health`,
 * `substance_use`, `sexual_health`, `reproductive` — useful when a
 * paramedic doesn't need that context for the current complaint.
 */
@Composable
fun PatientScreen(
    caseUlid: String,
    onOpenIntervention: () -> Unit,
    onOpenTimeline: () -> Unit,
    onOpenHandoff: () -> Unit,
    onPanicLogout: () -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
) {
    val ctx = LocalContext.current
    var view by remember { mutableStateOf<PatientView?>(null) }
    var hideSensitive by remember { mutableStateOf(false) }
    val activeCase by CaseVault.activeCase.collectAsState()
    val syncStatus by CaseVault.syncStatus.collectAsState()
    val queuedWrites by CaseVault.queuedWrites.collectAsState()

    LaunchedEffect(caseUlid) {
        view = EmergencyRepository.loadPatientView(caseUlid)
    }

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(modifier = Modifier.fillMaxSize().padding(contentPadding)) {
            EmergencyTopBar(
                operatorLabel = OperatorSession.operatorLabel(ctx),
                responderLabel = OperatorSession.responderLabel(ctx),
                syncStatus = SyncIndicatorState(syncStatus, queuedWrites.size),
                activeCaseShortLabel = activeCase?.caseUlid?.takeLast(6),
                onPanicLogout = onPanicLogout,
            )

            val v = view
            if (v == null) {
                Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(
                        text = "Loading patient view…",
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            } else {
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .verticalScroll(rememberScrollState())
                        .padding(horizontal = 24.dp, vertical = 16.dp),
                    verticalArrangement = Arrangement.spacedBy(20.dp),
                ) {
                    PatientHeader(
                        view = v,
                        autoGranted = activeCase?.autoGranted == true,
                        hideSensitive = hideSensitive,
                        onToggleSensitive = { hideSensitive = it },
                    )
                    CriticalCard(info = v.criticalInfo)
                    MedicationsCard(meds = v.activeMedications)
                    VitalsRow(vitals = v.recentVitals)
                    DiagnosesCard(diagnoses = v.activeDiagnoses)
                    ObservationsCard(observations = v.recentObservations)

                    Spacer(Modifier.height(80.dp)) // breathing room above nav bar
                }

                CaseNavBar(
                    selected = CaseTab.Patient,
                    onPatient = {},
                    onIntervention = onOpenIntervention,
                    onTimeline = onOpenTimeline,
                    onHandoff = onOpenHandoff,
                )
            }
        }
    }
}

@Composable
private fun PatientHeader(
    view: PatientView,
    autoGranted: Boolean,
    hideSensitive: Boolean,
    onToggleSensitive: (Boolean) -> Unit,
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(20.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = view.patientLabel,
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.SemiBold,
                )
                val demographic = listOfNotNull(
                    view.patientAge?.let { "$it y" },
                    view.patientSex,
                ).joinToString(" · ")
                if (demographic.isNotEmpty()) {
                    Text(
                        text = demographic,
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Text(
                    text = "Case ${view.caseUlid.takeLast(8)}  ·  open since ${formatTime(view.openedAtMs)}  ·  elapsed ${minutesSince(view.openedAtMs)}",
                    style = MonoStyle,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            Column(horizontalAlignment = Alignment.End) {
                if (autoGranted) {
                    StatusChip(
                        label = "Auto-granted via timeout",
                        tone = ChipTone.AutoGrant,
                    )
                    Spacer(Modifier.height(8.dp))
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = "Hide non-emergency data",
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.size(8.dp))
                    Switch(checked = hideSensitive, onCheckedChange = onToggleSensitive)
                }
            }
        }
    }
}

@Composable
private fun MedicationsCard(meds: List<MedicationEntry>) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Column(modifier = Modifier.padding(20.dp)) {
            CardTitle("Active medications", icon = Icons.Filled.MedicalServices)
            Spacer(Modifier.height(12.dp))
            meds.forEach { m ->
                Row(
                    modifier = Modifier.fillMaxWidth().padding(vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(modifier = Modifier.weight(1f)) {
                        Text(text = m.name, style = MaterialTheme.typography.titleMedium)
                        Text(
                            text = m.dose,
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Text(
                        text = m.lastTakenAtMs?.let { "Last: ${formatTime(it)}" } ?: "—",
                        style = MonoStyle,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.3f))
            }
        }
    }
}

@Composable
private fun VitalsRow(vitals: List<VitalSnapshot>) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Column(modifier = Modifier.padding(20.dp)) {
            CardTitle("Recent vitals (last ~30 min)", icon = Icons.Filled.MonitorHeart)
            Spacer(Modifier.height(12.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                vitals.forEach { snap ->
                    VitalCard(snap, modifier = Modifier.weight(1f))
                }
            }
        }
    }
}

@Composable
private fun VitalCard(snap: VitalSnapshot, modifier: Modifier = Modifier) {
    Card(
        modifier = modifier,
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
        ),
    ) {
        Column(modifier = Modifier.padding(12.dp)) {
            Text(
                text = snap.displayLabel,
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(4.dp))
            Row(verticalAlignment = Alignment.Bottom) {
                Text(
                    text = snap.latestValue,
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(Modifier.size(4.dp))
                Text(
                    text = snap.latestUnit,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(bottom = 6.dp),
                )
            }
            Spacer(Modifier.height(4.dp))
            Sparkline(snap.series, modifier = Modifier.fillMaxWidth().height(48.dp))
            Text(
                text = "Last: ${formatTime(snap.takenAtMs)}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun Sparkline(readings: List<VitalReading>, modifier: Modifier = Modifier) {
    if (readings.size < 2) {
        Box(modifier = modifier)
        return
    }
    val color = EmergencyPalette.RedBright
    Canvas(modifier = modifier) {
        val minV = readings.minOf { it.value }
        val maxV = readings.maxOf { it.value }
        val span = max(0.0001, maxV - minV)
        val w = size.width
        val h = size.height
        val path = Path()
        readings.forEachIndexed { i, r ->
            val x = w * (i.toFloat() / (readings.size - 1))
            val y = h * (1 - ((r.value - minV) / span)).toFloat()
            if (i == 0) path.moveTo(x, y) else path.lineTo(x, y)
        }
        drawPath(
            path = path,
            color = color,
            style = Stroke(width = 3f, cap = StrokeCap.Round),
        )
        // Highlight last point.
        val lastX = w
        val lastY = h * (1 - ((readings.last().value - minV) / span)).toFloat()
        drawCircle(
            color = color,
            radius = 5f,
            center = Offset(min(w, lastX) - 1f, lastY),
        )
    }
}

@Composable
private fun DiagnosesCard(diagnoses: List<String>) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Column(modifier = Modifier.padding(20.dp)) {
            CardTitle("Active diagnoses", icon = Icons.Filled.LocalHospital)
            Spacer(Modifier.height(12.dp))
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                diagnoses.forEach {
                    Text(
                        text = "• $it",
                        style = MaterialTheme.typography.bodyLarge,
                    )
                }
            }
        }
    }
}

@Composable
private fun ObservationsCard(observations: List<com.ohd.emergency.data.ObservationEntry>) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Column(modifier = Modifier.padding(20.dp)) {
            CardTitle("Recent observations", icon = Icons.Filled.History)
            Spacer(Modifier.height(12.dp))
            observations.forEach {
                Row(modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                    Text(
                        text = formatTime(it.timestampMs),
                        style = MonoStyle,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.fillMaxWidth(0.18f),
                    )
                    Text(
                        text = it.text,
                        style = MaterialTheme.typography.bodyLarge,
                    )
                }
            }
        }
    }
}

@Composable
private fun CardTitle(title: String, icon: androidx.compose.ui.graphics.vector.ImageVector) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        androidx.compose.material3.Icon(
            imageVector = icon,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = Modifier.size(20.dp),
        )
        Spacer(Modifier.size(8.dp))
        Text(text = title, style = MaterialTheme.typography.titleMedium)
    }
}

private val timeFmt = SimpleDateFormat("HH:mm:ss", Locale.getDefault())
private fun formatTime(ms: Long): String = timeFmt.format(Date(ms))

private fun minutesSince(ms: Long): String {
    val mins = ((System.currentTimeMillis() - ms) / 60_000).toInt()
    return if (mins < 60) "${mins}m" else "${mins / 60}h ${mins % 60}m"
}
