package com.ohd.connect.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdInput
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdColors

/**
 * Static library entry — name + a "common dose" pre-formatted for display.
 *
 * The screen renders each row as an [OhdListItem] with `name` as primary
 * and `commonDose` as secondary. Tapping a row hands the entry back to the
 * caller (today: a toast; once a real "user medication list" exists, an
 * actual add-to-on-hand storage write).
 */
data class MedicationLibraryEntry(val name: String, val commonDose: String)

private val LIBRARY: List<MedicationLibraryEntry> = listOf(
    MedicationLibraryEntry("Metformin", "500 mg · twice daily"),
    MedicationLibraryEntry("Lisinopril", "10 mg · once daily"),
    MedicationLibraryEntry("Atorvastatin", "20 mg · once daily"),
    MedicationLibraryEntry("Levothyroxine", "50 mcg · once daily"),
    MedicationLibraryEntry("Amlodipine", "5 mg · once daily"),
    MedicationLibraryEntry("Albuterol", "2 puffs · as needed"),
    MedicationLibraryEntry("Omeprazole", "20 mg · once daily"),
    MedicationLibraryEntry("Sertraline", "50 mg · once daily"),
    MedicationLibraryEntry("Losartan", "50 mg · once daily"),
    MedicationLibraryEntry("Hydrochlorothiazide", "25 mg · once daily"),
    MedicationLibraryEntry("Gabapentin", "300 mg · three times daily"),
    MedicationLibraryEntry("Vitamin D3", "2000 IU · once daily"),
    MedicationLibraryEntry("Omega-3", "1000 mg · once daily"),
    MedicationLibraryEntry("Magnesium", "400 mg · once daily"),
    MedicationLibraryEntry("Probiotic", "1 capsule · once daily"),
    MedicationLibraryEntry("Multivitamin", "1 tablet · once daily"),
    MedicationLibraryEntry("Vitamin B12", "1000 mcg · once daily"),
    MedicationLibraryEntry("Iron", "65 mg · once daily"),
    MedicationLibraryEntry("Calcium", "600 mg · twice daily"),
    MedicationLibraryEntry("Melatonin", "3 mg · at night"),
    MedicationLibraryEntry("Ibuprofen", "200 mg · as needed"),
    MedicationLibraryEntry("Acetaminophen", "500 mg · as needed"),
    MedicationLibraryEntry("Aspirin", "81 mg · once daily"),
    MedicationLibraryEntry("Diphenhydramine", "25 mg · as needed"),
    MedicationLibraryEntry("Loratadine", "10 mg · once daily"),
    MedicationLibraryEntry("Cetirizine", "10 mg · once daily"),
    MedicationLibraryEntry("Ranitidine", "150 mg · as needed"),
    MedicationLibraryEntry("Famotidine", "20 mg · as needed"),
    MedicationLibraryEntry("Naproxen", "220 mg · as needed"),
    MedicationLibraryEntry("Cyanocobalamin", "1000 mcg · once daily"),
)

/**
 * Medication library — searchable list of preset entries.
 *
 * Wired from `MedicationScreen.onOpenLibrary`. Tapping a row hands the
 * [MedicationLibraryEntry.name] back via [onPickEntry]; today the caller
 * surfaces `"Added X to on-hand"` and pops. When a real medication-list
 * persistence layer lands the callback will commit a row instead.
 *
 * The screen owns its own [OhdTopBar] (title + back arrow); no top-bar
 * action.
 */
@Composable
fun MedicationLibraryScreen(
    onBack: () -> Unit,
    onPickEntry: (MedicationLibraryEntry) -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
) {
    var query by remember { mutableStateOf("") }

    val filtered = remember(query) {
        val q = query.trim()
        if (q.isEmpty()) {
            LIBRARY
        } else {
            LIBRARY.filter { it.name.contains(q, ignoreCase = true) }
        }
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Medication library", onBack = onBack)

        // Search bar — 16 dp inset, sits above the list.
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            OhdInput(
                value = query,
                onValueChange = { query = it },
                placeholder = "Search medications…",
            )
        }

        LazyColumn(modifier = Modifier.fillMaxWidth()) {
            items(filtered, key = { it.name }) { entry ->
                OhdListItem(
                    primary = entry.name,
                    secondary = entry.commonDose,
                    meta = "+",
                    onClick = { onPickEntry(entry) },
                )
                OhdDivider()
            }
        }
    }
}
