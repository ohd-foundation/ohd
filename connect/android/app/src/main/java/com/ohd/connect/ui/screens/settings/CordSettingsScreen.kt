package com.ohd.connect.ui.screens.settings

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.Auth
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdCard
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdToggle
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TopBarAction
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * One row in the "Bring your own provider" section. Each provider has its
 * own copy + placeholder, but the persistence and status-pill logic are
 * identical — captured here so the body of the card stays compact.
 */
private data class ProviderRow(
    val id: String,
    val title: String,
    val help: String,
    val placeholder: String,
)

private val PROVIDERS: List<ProviderRow> = listOf(
    ProviderRow(
        id = "anthropic",
        title = "Anthropic",
        help = "Used for Claude models. Stored encrypted on this device.",
        placeholder = "sk-ant-…",
    ),
    ProviderRow(
        id = "openai",
        title = "OpenAI",
        help = "Used for GPT models. Stored encrypted on this device.",
        placeholder = "sk-…",
    ),
    ProviderRow(
        id = "gemini",
        title = "Google Gemini",
        help = "Used for Gemini models. Stored encrypted on this device.",
        placeholder = "AIza…",
    ),
)

/** Model the user can pick under "PREFERRED MODEL". */
private data class CordSettingsModel(val id: String, val provider: String)

// Static fallback list for the case where the Anthropic /v1/models endpoint
// is unreachable (no key yet, offline, rate-limited). Overridden at runtime
// by [AnthropicClient.listModels] when a key is configured.
private val SETTINGS_MODELS: List<CordSettingsModel> = listOf(
    CordSettingsModel("claude-sonnet-4-5", "Anthropic"),
    CordSettingsModel("claude-haiku-4-5", "Anthropic"),
    CordSettingsModel("gpt-4o-mini", "OpenAI"),
    CordSettingsModel("gemini-2.0-flash", "Google"),
)

/**
 * CORD settings — the landing destination from `Settings hub → CORD`.
 *
 * Replaces the previous direct-to-chat behaviour: the row now lands here so
 * the user can plug in their own provider API key, pick a preferred model,
 * or sign up for the future OHD-managed model. The top-bar "Open chat"
 * action navigates onward to [com.ohd.connect.ui.screens.cord.CordChatScreen]
 * — same surface the row used to hit directly.
 *
 * Body cards:
 *  1. OHD-managed model — invitation to the eventual SaaS offering.
 *  2. Bring your own provider — three API-key fields (Anthropic / OpenAI /
 *     Gemini) + a single-select preferred-model list. Keys are persisted via
 *     [Auth.saveCordApiKey]; the selected model reuses the existing
 *     `cord_selected_model` pref so the chat top-bar chip stays in sync.
 *  3. Local / offline — single toggle backing [Auth.cordStubResponsesEnabled].
 *  4. What gets sent — privacy paragraph.
 *  5. Debug — "Test API key" row that v1 just toasts; real ping wiring is
 *     deferred until a real provider client lands.
 */
@Composable
fun CordSettingsScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onOpenChat: () -> Unit,
) {
    val ctx = LocalContext.current

    // Provider keys — load once into a mutable map so each field has
    // independent state without dragging the whole composable.
    val keyState = remember {
        mutableStateOf(PROVIDERS.associate { it.id to Auth.loadCordApiKey(ctx, it.id) })
    }

    var selectedModel by remember { mutableStateOf(Auth.cordSelectedModel(ctx)) }
    var stubEnabled by remember { mutableStateOf(Auth.cordStubResponsesEnabled(ctx)) }
    var snackbar by remember { mutableStateOf<String?>(null) }
    // Live Anthropic catalog — overrides the static fallback once we have a key.
    var liveModels by remember { mutableStateOf<List<CordSettingsModel>?>(null) }
    val anthropicKey = keyState.value["anthropic"].orEmpty()
    LaunchedEffect(anthropicKey) {
        if (anthropicKey.isEmpty()) {
            liveModels = null
            return@LaunchedEffect
        }
        com.ohd.connect.data.AnthropicClient.listModels(anthropicKey).onSuccess { models ->
            liveModels = models.map { CordSettingsModel(it.id, "Anthropic") }
        }
    }
    val effectiveModels: List<CordSettingsModel> = liveModels ?: SETTINGS_MODELS

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(
            title = "CORD",
            onBack = onBack,
            action = TopBarAction(label = "Open chat", onClick = onOpenChat),
        )

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            ManagedModelCard(
                onSignUp = {
                    val intent = Intent(
                        Intent.ACTION_VIEW,
                        Uri.parse("https://ohd.dev/roadmap.html#cord-saas"),
                    ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    val ok = runCatching { ctx.startActivity(intent) }.isSuccess
                    if (!ok) snackbar = "No browser available"
                },
            )

            BringYourOwnProviderCard(
                keys = keyState.value,
                onKeyChange = { provider, value ->
                    keyState.value = keyState.value.toMutableMap().also { it[provider] = value }
                    Auth.saveCordApiKey(ctx, provider, value)
                },
                selectedModel = selectedModel,
                onModelChange = { id ->
                    selectedModel = id
                    Auth.saveCordSelectedModel(ctx, id)
                },
                models = effectiveModels,
            )

            LocalOfflineCard(
                enabled = stubEnabled,
                onToggle = { v ->
                    stubEnabled = v
                    Auth.setCordStubResponses(ctx, v)
                },
            )

            PrivacyCard()

            DebugCard(
                onTest = { snackbar = "Provider ping coming soon" },
            )

            if (snackbar != null) {
                Text(
                    text = snackbar!!,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Ink,
                )
            }
        }
    }
}

@Composable
private fun ManagedModelCard(onSignUp: () -> Unit) {
    OhdCard(title = "OHD-managed model") {
        Text(
            text = "Skip the API-key dance. We run a Gemini-backed model with " +
                "MCP access to your data — your grant rules apply.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            lineHeight = 19.sp,
            color = OhdColors.Ink,
        )
        Text(
            text = "Pricing TBA — we'll announce when general availability lands.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            color = OhdColors.Muted,
        )
        Text(
            text = "Currently in private beta. Sign up for the waitlist if you " +
                "want access when invites open up.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            color = OhdColors.Muted,
        )
        Spacer(modifier = Modifier.height(4.dp))
        OhdButton(
            label = "Sign up for early access",
            onClick = onSignUp,
            variant = OhdButtonVariant.Primary,
        )
    }
}

@Composable
private fun BringYourOwnProviderCard(
    keys: Map<String, String>,
    onKeyChange: (String, String) -> Unit,
    selectedModel: String,
    onModelChange: (String) -> Unit,
    models: List<CordSettingsModel> = SETTINGS_MODELS,
) {
    OhdCard(title = "Bring your own provider") {
        PROVIDERS.forEachIndexed { idx, p ->
            if (idx > 0) Spacer(modifier = Modifier.height(8.dp))
            ProviderKeyBlock(
                provider = p,
                value = keys[p.id].orEmpty(),
                onValueChange = { onKeyChange(p.id, it) },
            )
        }

        Spacer(modifier = Modifier.height(8.dp))
        // Section header is meant to sit at row level (not card-padded), but
        // inside the card the same uppercase/letter-spaced styling reads as a
        // sub-divider, which is exactly what we want here.
        OhdSectionHeader(text = "Preferred model")
        models.forEach { model ->
            ModelSelectRow(
                label = model.id,
                meta = model.provider,
                selected = model.id == selectedModel,
                onClick = { onModelChange(model.id) },
            )
        }
    }
}

@Composable
private fun ProviderKeyBlock(
    provider: ProviderRow,
    value: String,
    onValueChange: (String) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = provider.title,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 14.sp,
                color = OhdColors.Ink,
                modifier = Modifier.weight(1f),
            )
            StatusPill(isSet = value.isNotEmpty())
        }
        Text(
            text = provider.help,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            color = OhdColors.Muted,
        )
        MaskedInput(
            value = value,
            onValueChange = onValueChange,
            placeholder = provider.placeholder,
        )
    }
}

/**
 * Password-style input. `OhdInput` doesn't support a mask param, so we
 * inline a 44 dp pill with a [PasswordVisualTransformation] over a
 * [BasicTextField]. Visual rules match `OhdInput` (8 dp radius, 1.5 dp
 * `ohd-line` border, 14 sp body text).
 */
@Composable
private fun MaskedInput(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
) {
    val shape = RoundedCornerShape(8.dp)
    val textStyle = TextStyle(
        fontFamily = OhdBody,
        fontWeight = FontWeight.W400,
        fontSize = 14.sp,
        color = OhdColors.Ink,
    )
    Row(
        modifier = Modifier
            .height(44.dp)
            .fillMaxWidth()
            .background(OhdColors.Bg, shape)
            .border(BorderStroke(1.5.dp, OhdColors.Line), shape)
            .padding(horizontal = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BasicTextField(
            value = value,
            onValueChange = onValueChange,
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            textStyle = textStyle,
            cursorBrush = SolidColor(OhdColors.Ink),
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Password,
            ),
            visualTransformation = PasswordVisualTransformation(),
            decorationBox = { inner ->
                if (value.isEmpty()) {
                    Text(
                        text = placeholder,
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 14.sp,
                        color = OhdColors.Muted,
                    )
                }
                inner()
            },
        )
    }
}

@Composable
private fun StatusPill(isSet: Boolean) {
    val (label, bg, fg) = if (isSet) {
        Triple("Set", OhdColors.Success.copy(alpha = 0.12f), OhdColors.Success)
    } else {
        Triple("Not configured", OhdColors.LineSoft, OhdColors.Muted)
    }
    Box(
        modifier = Modifier
            .background(bg, RoundedCornerShape(12.dp))
            .padding(horizontal = 8.dp, vertical = 2.dp),
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 11.sp,
            color = fg,
        )
    }
}

@Composable
private fun ModelSelectRow(
    label: String,
    meta: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
    OhdListItem(
        primary = label,
        meta = meta,
        leading = {
            Box(
                modifier = Modifier
                    .size(18.dp)
                    .let { base ->
                        if (selected) base.background(OhdColors.Red, CircleShape)
                        else base.border(1.5.dp, OhdColors.Line, CircleShape)
                    },
            )
        },
        onClick = onClick,
    )
}

@Composable
private fun LocalOfflineCard(
    enabled: Boolean,
    onToggle: (Boolean) -> Unit,
) {
    OhdCard(title = "Local / offline") {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = "Stub responses without calling a provider",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 14.sp,
                    color = OhdColors.Ink,
                )
                Text(
                    text = "Useful while the real LLM wiring is in flux. The chat " +
                        "screen echoes a canned response with a chart card. Untoggle " +
                        "once you've configured a real API key above.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    lineHeight = 18.sp,
                    color = OhdColors.Muted,
                )
            }
            OhdToggle(checked = enabled, onCheckedChange = onToggle)
        }
    }
}

@Composable
private fun PrivacyCard() {
    OhdCard(title = "What gets sent") {
        Text(
            text = "When you chat with CORD, the prompt + a scoped subset of your " +
                "OHD events (per the active CORD grant) leave this device for the " +
                "provider you chose. The provider sees your data only for the " +
                "duration of one request. We do not log prompts on our side. With " +
                "the OHD-managed model, prompts route through a managed MCP " +
                "server — the same provider boundary applies.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            lineHeight = 18.sp,
            color = OhdColors.Muted,
        )
    }
}

@Composable
private fun DebugCard(onTest: () -> Unit) {
    OhdCard(title = "Debug") {
        OhdListItem(
            primary = "Test API key",
            meta = "›",
            onClick = onTest,
        )
    }
}

