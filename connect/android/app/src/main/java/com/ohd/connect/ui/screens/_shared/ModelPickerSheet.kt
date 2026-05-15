package com.ohd.connect.ui.screens._shared

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.launch

/** A model the user can pick in the CORD chat top-bar chip. */
data class CordModel(val id: String, val label: String, val sub: String)

/**
 * Static fallback list — used only when the live Anthropic catalog
 * (`AnthropicClient.listModels`) is unavailable (no API key, offline,
 * rate-limited). The sheet swaps to live results the moment the user
 * has a key configured.
 */
val CORD_MODELS: List<CordModel> = listOf(
    CordModel("claude-sonnet-4-5", "claude-sonnet-4-5", "Anthropic · cloud"),
    CordModel("claude-haiku-4-5", "claude-haiku-4-5", "Anthropic · cloud · fast"),
    CordModel("gpt-4o-mini", "gpt-4o-mini", "OpenAI · cloud"),
)

/**
 * Modal bottom sheet for swapping the active CORD model.
 *
 * Selecting a row dismisses the sheet and invokes [onPick] with the
 * picked model's id. The caller persists the choice via
 * [com.ohd.connect.data.Auth.saveCordSelectedModel] and re-renders the
 * top-bar chip with the new label.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ModelPickerSheet(
    selectedId: String,
    onDismiss: () -> Unit,
    onPick: (String) -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    val ctx = androidx.compose.ui.platform.LocalContext.current

    // Live catalog — replaces the fallback list as soon as we have a key.
    // Same call CordSettingsScreen makes; results cache for the session.
    var liveModels by remember { mutableStateOf<List<CordModel>?>(null) }
    LaunchedEffect(Unit) {
        val key = com.ohd.connect.data.Auth.loadCordApiKey(ctx, "anthropic")
        if (key.isEmpty()) return@LaunchedEffect
        com.ohd.connect.data.AnthropicClient.listModels(key).onSuccess { rows ->
            liveModels = rows.map { CordModel(it.id, it.displayName.ifBlank { it.id }, "Anthropic · cloud") }
        }
    }

    val models = liveModels ?: CORD_MODELS

    val pickThen: (String) -> Unit = { id ->
        scope.launch { sheetState.hide() }.invokeOnCompletion {
            onDismiss()
            onPick(id)
        }
    }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = OhdColors.Bg,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = "MODEL",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 11.sp,
                letterSpacing = 2.sp,
                color = OhdColors.Muted,
                modifier = Modifier.padding(horizontal = 4.dp, vertical = 8.dp),
            )
            models.forEach { model ->
                ModelRow(
                    model = model,
                    isSelected = model.id == selectedId,
                    onClick = { pickThen(model.id) },
                )
            }
            Spacer(Modifier.height(8.dp))
        }
    }
}

@Composable
private fun ModelRow(
    model: CordModel,
    isSelected: Boolean,
    onClick: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(56.dp)
            .clickable { onClick() }
            .padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        // Radio affordance — filled ohd-red disc when selected, hollow
        // 1.5 dp ohd-line ring when not.
        Box(
            modifier = Modifier
                .size(18.dp)
                .let { base ->
                    if (isSelected) {
                        base.background(OhdColors.Red, CircleShape)
                    } else {
                        base.border(1.5.dp, OhdColors.Line, CircleShape)
                    }
                },
        )
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = model.label,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 14.sp,
                color = OhdColors.Ink,
            )
            Text(
                text = model.sub,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
        }
    }
}
