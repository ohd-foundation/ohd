package com.ohd.connect.ui.screens.cord

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
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.Auth
import com.ohd.connect.data.CordRunner
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.screens._shared.ModelPickerSheet
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdDisplay
import kotlinx.coroutines.launch

private sealed interface ChatMessage {
    data class User(val text: String) : ChatMessage
    data class Assistant(
        val text: String,
        val chart: ChatChart? = null,
    ) : ChatMessage
    data class Error(val text: String) : ChatMessage
}

private data class ChatChart(
    val title: String,
    val caption: String,
)

/**
 * Personal CORD chat — Pencil `NsOBH.png`, spec §4.11.
 *
 * Custom in-line top bar (NOT [com.ohd.connect.ui.components.OhdTopBar]
 * because it needs the model-selector chip on the right).
 *
 * Conversation seeded with the two-turn sample exchange from the spec.
 * Sending a new message appends a stub assistant reply per spec §4.11
 * ("I'm offline in this build…").
 */
@Composable
fun CordChatScreen(
    onBack: () -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
) {
    val ctx = LocalContext.current
    var selectedModel by remember { mutableStateOf(Auth.cordSelectedModel(ctx)) }
    var pickerOpen by remember { mutableStateOf(false) }

    // Honour external changes to the persisted model (e.g. another screen
    // swaps it) on re-entry.
    LaunchedEffect(Unit) {
        selectedModel = Auth.cordSelectedModel(ctx)
    }

    val messages = remember {
        mutableStateListOf<ChatMessage>(
            ChatMessage.User("What's been happening with my sleep lately?"),
            ChatMessage.Assistant(
                text = "Your average sleep has dropped from 7.2h to 5.8h over 3 " +
                    "weeks — a 19% decline. This correlates with later " +
                    "bedtimes visible in your activity log.",
                chart = ChatChart(
                    title = "Sleep duration · last 30 days",
                    caption = "Avg 5.8h  ↓ from 7.2h",
                ),
            ),
            ChatMessage.User("Could that explain why I've felt so tired?"),
            ChatMessage.Assistant(
                text = "Almost certainly. On days following less than 6h of sleep " +
                    "your fatigue score averaged 3.8/5, resting heart rate " +
                    "was elevated by 6 bpm, and you logged fewer than 4,000 " +
                    "steps. Your body is in consistent recovery mode.",
            ),
        )
    }
    var draft by remember { mutableStateOf("") }
    var thinking by remember { mutableStateOf(false) }
    var toolStatus by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        // Top bar — custom because of the model-selector chip on the right.
        CordTopBar(
            modelLabel = selectedModel,
            onBack = onBack,
            onPickModel = { pickerOpen = true },
        )

        // Thread.
        Column(
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // Notice row — centered.
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Row(
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(
                        imageVector = OhdIcons.Database,
                        contentDescription = null,
                        tint = OhdColors.Muted,
                        modifier = Modifier.size(14.dp),
                    )
                    Text(
                        text = "12,847 events · analysing your data",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 11.sp,
                        color = OhdColors.Muted,
                    )
                }
            }

            messages.forEach { msg ->
                when (msg) {
                    is ChatMessage.User -> UserBubble(text = msg.text)
                    is ChatMessage.Assistant -> AssistantRow(message = msg)
                    is ChatMessage.Error -> ErrorRow(text = msg.text)
                }
            }

            if (thinking) {
                ThinkingRow(label = toolStatus)
            }
        }

        // Input bar.
        CordInputBar(
            draft = draft,
            onDraftChange = { draft = it },
            onSend = {
                val trimmed = draft.trim()
                if (trimmed.isEmpty() || thinking) return@CordInputBar
                messages.add(ChatMessage.User(trimmed))
                draft = ""

                // Stub-mode bypass — kept so the user can still demo the UI
                // without burning API credits.
                if (Auth.cordStubResponsesEnabled(ctx)) {
                    messages.add(
                        ChatMessage.Assistant(
                            text = "(stub mode) Echo: $trimmed",
                        ),
                    )
                    return@CordInputBar
                }

                if (!Auth.isCordApiKeySet(ctx, "anthropic")) {
                    messages.add(
                        ChatMessage.Error(
                            text = "Set your Anthropic key in Settings → CORD to use the chat.",
                        ),
                    )
                    return@CordInputBar
                }

                thinking = true
                toolStatus = null
                val historySnapshot = messages.mapNotNull { msg ->
                    when (msg) {
                        is ChatMessage.User -> CordRunner.UiMessage("user", msg.text)
                        is ChatMessage.Assistant -> CordRunner.UiMessage("assistant", msg.text)
                        is ChatMessage.Error -> null
                    }
                }
                scope.launch {
                    val result = CordRunner.ask(
                        ctx = ctx,
                        history = historySnapshot,
                        onAssistantText = { text ->
                            messages.add(ChatMessage.Assistant(text = text))
                            toolStatus = null
                        },
                        onToolUse = { name ->
                            toolStatus = "Running $name…"
                        },
                    )
                    result.onFailure { err ->
                        messages.add(
                            ChatMessage.Error(
                                text = err.message ?: "Request failed.",
                            ),
                        )
                    }
                    thinking = false
                    toolStatus = null
                }
            },
        )
    }

    // Model picker sheet — opens when the chip is tapped. Persists the
    // user's choice via Auth so the next CORD entry remembers it.
    if (pickerOpen) {
        ModelPickerSheet(
            selectedId = selectedModel,
            onDismiss = { pickerOpen = false },
            onPick = { id ->
                selectedModel = id
                Auth.saveCordSelectedModel(ctx, id)
            },
        )
    }
}

@Composable
private fun CordTopBar(
    modelLabel: String,
    onBack: () -> Unit,
    onPickModel: () -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(52.dp)
                .background(OhdColors.Bg)
                .padding(horizontal = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                modifier = Modifier
                    .size(36.dp)
                    .clickable { onBack() },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = OhdIcons.ArrowLeft,
                    contentDescription = "Back",
                    tint = OhdColors.Ink,
                    modifier = Modifier.size(20.dp),
                )
            }

            Text(
                text = "CORD",
                fontFamily = OhdDisplay,
                fontWeight = FontWeight.W300,
                fontSize = 17.sp,
                color = OhdColors.Ink,
                modifier = Modifier.weight(1f),
                textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )

            // Model chip.
            Row(
                modifier = Modifier
                    .background(OhdColors.BgElevated, RoundedCornerShape(12.dp))
                    .clickable { onPickModel() }
                    .padding(horizontal = 10.dp, vertical = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    text = modelLabel,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 11.sp,
                    color = OhdColors.Muted,
                )
                Icon(
                    imageVector = OhdIcons.ChevronDown,
                    contentDescription = null,
                    tint = OhdColors.Muted,
                    modifier = Modifier.size(12.dp),
                )
            }
        }
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(OhdColors.Line),
        )
    }
}

@Composable
private fun UserBubble(text: String) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.End,
    ) {
        Box(
            modifier = Modifier
                .widthIn(max = 240.dp)
                .background(
                    OhdColors.Ink,
                    RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp, bottomEnd = 4.dp, bottomStart = 16.dp),
                )
                .padding(12.dp),
        ) {
            Text(
                text = text,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 14.sp,
                lineHeight = 21.sp,
                color = OhdColors.White,
            )
        }
    }
}

@Composable
private fun AssistantRow(message: ChatMessage.Assistant) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.Top,
    ) {
        // 28 dp red circle avatar.
        Box(
            modifier = Modifier
                .size(28.dp)
                .background(OhdColors.Red, CircleShape),
        )
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(
                        OhdColors.BgElevated,
                        RoundedCornerShape(topStart = 4.dp, topEnd = 16.dp, bottomEnd = 16.dp, bottomStart = 16.dp),
                    )
                    .padding(12.dp),
            ) {
                Text(
                    text = message.text,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 14.sp,
                    lineHeight = 21.sp,
                    color = OhdColors.Ink,
                )
            }

            if (message.chart != null) {
                AssistantChartCard(chart = message.chart)
            }
        }
    }
}

@Composable
private fun AssistantChartCard(chart: ChatChart) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(OhdColors.Bg, RoundedCornerShape(8.dp))
            .border(BorderStroke(1.dp, OhdColors.LineSoft), RoundedCornerShape(8.dp))
            .padding(10.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text(
            text = chart.title,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 11.sp,
            color = OhdColors.Muted,
        )
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(60.dp)
                .background(OhdColors.LineSoft, RoundedCornerShape(4.dp)),
        )
        Text(
            text = chart.caption,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 11.sp,
            color = OhdColors.Muted,
        )
    }
}

@Composable
private fun ThinkingRow(label: String?) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(
            modifier = Modifier
                .size(28.dp)
                .background(OhdColors.Red, CircleShape),
        )
        Box(
            modifier = Modifier
                .background(
                    OhdColors.BgElevated,
                    RoundedCornerShape(topStart = 4.dp, topEnd = 16.dp, bottomEnd = 16.dp, bottomStart = 16.dp),
                )
                .padding(horizontal = 12.dp, vertical = 10.dp),
        ) {
            Text(
                text = label ?: "Thinking…",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 13.sp,
                color = OhdColors.Muted,
            )
        }
    }
}

@Composable
private fun ErrorRow(text: String) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.Center,
    ) {
        Box(
            modifier = Modifier
                .background(OhdColors.BgElevated, RoundedCornerShape(8.dp))
                .border(BorderStroke(1.dp, OhdColors.Red), RoundedCornerShape(8.dp))
                .padding(horizontal = 12.dp, vertical = 8.dp),
        ) {
            Text(
                text = text,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Red,
            )
        }
    }
}

@Composable
private fun CordInputBar(
    draft: String,
    onDraftChange: (String) -> Unit,
    onSend: () -> Unit,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(OhdColors.Line),
        )
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(OhdColors.Bg)
                .padding(horizontal = 12.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // Input pill.
            Box(
                modifier = Modifier
                    .weight(1f)
                    .height(40.dp)
                    .background(OhdColors.BgElevated, RoundedCornerShape(20.dp))
                    .padding(horizontal = 12.dp),
                contentAlignment = Alignment.CenterStart,
            ) {
                if (draft.isEmpty()) {
                    Text(
                        text = "Ask anything about your health…",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 14.sp,
                        color = OhdColors.Muted,
                    )
                }
                BasicTextField(
                    value = draft,
                    onValueChange = onDraftChange,
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                    textStyle = TextStyle(
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 14.sp,
                        color = OhdColors.Ink,
                    ),
                    cursorBrush = SolidColor(OhdColors.Ink),
                )
            }

            // 36 dp red circle send button.
            Box(
                modifier = Modifier
                    .size(36.dp)
                    .background(OhdColors.Red, CircleShape)
                    .clickable { onSend() },
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = OhdIcons.ArrowUp,
                    contentDescription = "Send",
                    tint = OhdColors.White,
                    modifier = Modifier.size(18.dp),
                )
            }
        }
    }
}
