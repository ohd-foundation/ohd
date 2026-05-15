package com.ohd.connect.ui.screens

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.EventFilter
import com.ohd.connect.data.OhdEvent
import com.ohd.connect.data.OpenFoodFacts
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdInput
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Tri-state for the OpenFoodFacts barcode lookup pipeline.
 *
 * The lookup runs in a [LaunchedEffect] keyed on the search query — when the
 * user keeps typing, the previous coroutine is cancelled, so we don't need a
 * "Cancelled" state.
 */
private sealed interface RemoteLookupState {
    data object Idle : RemoteLookupState
    data object Loading : RemoteLookupState
    data object NotFound : RemoteLookupState
    data class Error(val message: String?) : RemoteLookupState
}

/** EAN-8 / UPC-A / EAN-13 all fall inside [8,13] digits. */
private val BARCODE_REGEX = Regex("^\\d{8,13}$")

/**
 * Food v3 — Search active — Pencil `yBPJe.png`, spec §4.7.
 *
 * Same nutrition panel as [FoodScreen] (reused via [FoodNutritionPanel]),
 * then a 2-column search row (44 dp barcode button + autofocused input) and
 * a results list filtered from [FoodDictionary] by query.
 *
 * When the query matches [BARCODE_REGEX] and no local entry matches, the
 * screen kicks off an [OpenFoodFacts.lookup] call. The result (if any) is
 * rendered above the local list under an "ONLINE — OPENFOODFACTS" header.
 *
 * Tapping a result invokes [onPickFood] — wired by the navigation graph to
 * push [FoodDetailScreen]. The 44×44 scan button on the left of the search
 * row now pops back to [FoodScreen] (camera view) via [onScanReturn] —
 * previously it was a no-op stub.
 */
@Composable
fun FoodSearchScreen(
    onBack: () -> Unit,
    onScanReturn: () -> Unit,
    onPickFood: (FoodItem) -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
    initialQuery: String = "",
) {
    var query by remember { mutableStateOf(initialQuery) }
    val results = remember(query) { searchFoodDictionary(query) }

    // Remote OFF lookup state — see [RemoteLookupState] kdoc.
    var remoteResults by remember { mutableStateOf<List<FoodItem>>(emptyList()) }
    var remoteState by remember { mutableStateOf<RemoteLookupState>(RemoteLookupState.Idle) }

    LaunchedEffect(query) {
        val trimmed = query.trim()
        if (trimmed.isBlank()) {
            remoteResults = emptyList()
            remoteState = RemoteLookupState.Idle
            return@LaunchedEffect
        }
        val isBarcode = BARCODE_REGEX.matches(trimmed)
        val localEmpty = results.isEmpty()
        if (isBarcode) {
            // Barcode path — cache shortcut, single-product lookup.
            OpenFoodFacts.cache[trimmed]?.let { cached ->
                remoteResults = listOf(cached)
                remoteState = RemoteLookupState.Idle
                return@LaunchedEffect
            }
            remoteState = RemoteLookupState.Loading
            remoteResults = emptyList()
            try {
                val item = OpenFoodFacts.lookup(trimmed)
                if (item != null) {
                    remoteResults = listOf(item)
                    remoteState = RemoteLookupState.Idle
                } else {
                    remoteState = RemoteLookupState.NotFound
                }
            } catch (t: Throwable) {
                remoteState = RemoteLookupState.Error(t.message)
            }
        } else {
            // Free-text path — only fire when the local dictionary returns
            // nothing useful, and only after the user has typed something
            // discriminating enough to be worth a network call. Three chars
            // is the OFF "min query length" sweet spot — shorter than that
            // and we'd see massive (unhelpful) result sets.
            if (trimmed.length < 3 || !localEmpty) {
                remoteResults = emptyList()
                remoteState = RemoteLookupState.Idle
                return@LaunchedEffect
            }
            // Tiny debounce so we don't fire on every keystroke.
            kotlinx.coroutines.delay(350)
            remoteState = RemoteLookupState.Loading
            remoteResults = emptyList()
            try {
                val items = OpenFoodFacts.search(trimmed, pageSize = 10)
                if (items.isNotEmpty()) {
                    remoteResults = items
                    remoteState = RemoteLookupState.Idle
                } else {
                    remoteState = RemoteLookupState.NotFound
                }
            } catch (t: Throwable) {
                remoteState = RemoteLookupState.Error(t.message)
            }
        }
    }

    // Re-fetch the same `food.eaten` slice as FoodScreen so the panel
    // reflects today's totals while the user is mid-search.
    var todaysFoods by remember { mutableStateOf<List<OhdEvent>>(emptyList()) }
    LaunchedEffect(Unit) {
        todaysFoods = StorageRepository
            .queryEvents(
                EventFilter(
                    fromMs = startOfTodayMs(),
                    eventTypesIn = listOf(FOOD_EATEN_EVENT_TYPE),
                    limit = 100,
                ),
            )
            .getOrNull()
            ?: emptyList()
    }
    val totals = remember(todaysFoods) { aggregateMacros(todaysFoods) }

    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(Unit) {
        // Best-effort autofocus on entry — matches the focused-input visual
        // in the Pencil export.
        runCatching { focusRequester.requestFocus() }
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Food", onBack = onBack)

        FoodNutritionPanel(totals = totals)

        // Search row.
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // 44 × 44 barcode button — pops back to FoodScreen (camera view).
            Box(
                modifier = Modifier
                    .size(44.dp)
                    .background(OhdColors.BgElevated, RoundedCornerShape(8.dp))
                    .border(BorderStroke(1.dp, OhdColors.Line), RoundedCornerShape(8.dp))
                    .clickable { onScanReturn() },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = OhdIcons.ScanBarcode,
                    contentDescription = "Scan barcode",
                    tint = OhdColors.Muted,
                    modifier = Modifier.size(20.dp),
                )
            }
            // Autofocused input.
            OhdInput(
                value = query,
                onValueChange = { query = it },
                placeholder = "Oat porridge…",
                modifier = Modifier
                    .weight(1f)
                    .focusRequester(focusRequester),
            )
        }

        LazyColumn(modifier = Modifier.fillMaxWidth()) {
            // Online section — shown only when we have remote results or
            // we're mid-flight / errored on a barcode-looking query.
            val showOnlineSection = remoteResults.isNotEmpty() ||
                remoteState != RemoteLookupState.Idle
            if (showOnlineSection) {
                item(key = "online-header") {
                    OhdSectionHeader(text = "ONLINE — OPENFOODFACTS")
                }
                when (val s = remoteState) {
                    RemoteLookupState.Loading -> item(key = "online-loading") {
                        RemoteStatusRow(
                            text = "Looking up barcode ${query.trim()} on OpenFoodFacts…",
                            showSpinner = true,
                        )
                    }
                    RemoteLookupState.NotFound -> item(key = "online-notfound") {
                        RemoteStatusRow(
                            text = "Couldn't find barcode ${query.trim()}. " +
                                "Try searching by name instead, or build a custom form.",
                        )
                    }
                    is RemoteLookupState.Error -> item(key = "online-error") {
                        RemoteStatusRow(
                            text = "Lookup failed: ${s.message ?: "(unknown)"}. " +
                                "Try again or search by name.",
                        )
                    }
                    RemoteLookupState.Idle -> Unit
                }
                itemsIndexed(
                    remoteResults,
                    key = { _, item -> "remote-${item.name}" },
                ) { index, item ->
                    OhdListItem(
                        primary = item.name,
                        secondary = "${item.per100g.kcal} kcal / 100g · ${item.source}",
                        meta = "›",
                        onClick = { onPickFood(item) },
                    )
                    if (index < remoteResults.lastIndex) {
                        OhdDivider()
                    }
                }
            }

            // Local in-app dictionary section.
            item(key = "local-header") {
                OhdSectionHeader(text = "RESULTS")
            }
            itemsIndexed(results, key = { _, item -> "local-${item.name}" }) { index, item ->
                // Tapping a result no longer logs directly — it opens the
                // FoodDetailScreen so the user can pick amount/unit before
                // committing the event.
                OhdListItem(
                    primary = item.name,
                    secondary = "${item.per100g.kcal} kcal / 100g · ${item.source}",
                    meta = "›",
                    onClick = { onPickFood(item) },
                )
                if (index < results.lastIndex) {
                    OhdDivider()
                }
            }
        }
    }
}

/**
 * Small muted status row used for the OpenFoodFacts loading / not-found /
 * error states. Stays visually quiet — mirrors the muted-text treatment we
 * use elsewhere for non-critical status under section headers.
 */
@Composable
private fun RemoteStatusRow(text: String, showSpinner: Boolean = false) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (showSpinner) {
            CircularProgressIndicator(
                modifier = Modifier.size(14.dp),
                color = OhdColors.Muted,
                strokeWidth = 2.dp,
            )
        }
        Text(
            text = text,
            color = OhdColors.Muted,
            fontFamily = OhdBody,
            fontSize = 13.sp,
        )
    }
}
