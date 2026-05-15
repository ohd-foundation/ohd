package com.ohd.connect.data

/**
 * Drives the Anthropic tool-use loop on behalf of CORD chat.
 *
 * The runner owns the "send → parse → maybe run tools → loop" cycle so the
 * Compose layer just hands in the conversation history and listens for
 * assistant-text events. The loop is bounded ([MAX_TOOL_ROUNDS]) to prevent
 * a misbehaving model from churning forever against the local store.
 */

import android.content.Context
import android.util.Log
import org.json.JSONObject

object CordRunner {

    private const val TAG = "OhdConnect.CordRunner"

    /** Hard cap on tool-use rounds per `ask` call. */
    private const val MAX_TOOL_ROUNDS = 8

    /**
     * Public chat shape. Mirrors the Compose-side `ChatMessage` but kept here
     * so [CordRunner] doesn't reach up into the UI layer.
     *
     *   role = "user" | "assistant"
     */
    data class UiMessage(val role: String, val text: String)

    private val SYSTEM_PROMPT = """
        You are CORD, the OHD personal-data agent. You have read-only access to
        the user's own on-device health and lifestyle event database via tools.

        Workflow:
          1. If you don't know what data the user has, call `describe_data`
             first — it returns total counts, event types, and the date range.
          2. For any time-relative question ("today", "last week", "since
             March"), call `now` once to anchor the clock + time zone.
          3. Use `query_events` to read rows. Prefer `event_type_prefix` for
             whole families (e.g. `intake.` for all nutrition channels). Cap
             `limit` to what you actually need — 50 rows is usually plenty.

        Replies should be short, plain, and quote concrete numbers from the
        events you read. Don't hedge with "I don't have data" until you've
        actually queried for it.
    """.trimIndent()

    /**
     * Run the tool-use loop. Returns `Result.success(Unit)` once the model
     * stops calling tools; `Result.failure` carries the first hard error.
     *
     *   [history]          — entire chat so far, oldest first. Last entry
     *                        should be the user's just-typed message.
     *   [onAssistantText]  — invoked for every assistant-text block, in
     *                        order. Compose appends each as a chat row.
     *   [onToolUse]        — invoked before each tool execution; the UI uses
     *                        it to swap the typing indicator for a "calling
     *                        <tool>" status.
     */
    suspend fun ask(
        ctx: Context,
        history: List<UiMessage>,
        onAssistantText: (String) -> Unit,
        onToolUse: (toolName: String) -> Unit = {},
    ): Result<Unit> {
        val apiKey = Auth.loadCordApiKey(ctx, "anthropic")
        if (apiKey.isBlank()) {
            return Result.failure(IllegalStateException("Anthropic API key not set"))
        }
        val model = resolveModelId(Auth.cordSelectedModel(ctx))

        // Seed the message list from the chat history. Tool calls and tool
        // results get appended to this list as the loop progresses; the
        // model needs to see them in order on every round.
        val messages = ArrayList<AnthropicClient.Msg>(history.size + 4)
        for (m in history) {
            messages += AnthropicClient.Msg(
                role = m.role,
                content = listOf(AnthropicClient.ContentBlock.Text(m.text)),
            )
        }

        var rounds = 0
        while (rounds < MAX_TOOL_ROUNDS) {
            rounds += 1
            val res = AnthropicClient.messages(
                apiKey = apiKey,
                model = model,
                system = SYSTEM_PROMPT,
                messages = messages,
                tools = CordTools.tools(ctx),
            ).getOrElse { return Result.failure(it) }

            // Emit every text block before deciding whether to loop. This way
            // the user sees the model's reasoning even when it follows up
            // with a tool call.
            for (block in res.content) {
                if (block is AnthropicClient.ContentBlock.Text && block.text.isNotBlank()) {
                    onAssistantText(block.text)
                }
            }

            if (res.stopReason != "tool_use") {
                return Result.success(Unit)
            }

            // Append the assistant turn verbatim, then a user-role turn that
            // pairs each tool_use id with a tool_result block.
            messages += AnthropicClient.Msg(role = "assistant", content = res.content)
            val toolResults = ArrayList<AnthropicClient.ContentBlock>()
            for (block in res.content) {
                if (block !is AnthropicClient.ContentBlock.ToolUse) continue
                onToolUse(block.name)
                val output = runCatching { CordTools.execute(ctx, block.name, block.input) }
                    .getOrElse {
                        Log.w(TAG, "Tool ${block.name} threw", it)
                        JSONObject().put("error", it.message ?: "tool failed").toString()
                    }
                val isError = output.startsWith("{\"error\":")
                toolResults += AnthropicClient.ContentBlock.ToolResult(
                    toolUseId = block.id,
                    content = output,
                    isError = isError,
                )
            }
            if (toolResults.isEmpty()) {
                // stop_reason said tool_use but no parsed tool blocks — bail
                // out rather than spin.
                return Result.success(Unit)
            }
            messages += AnthropicClient.Msg(role = "user", content = toolResults)
        }

        onAssistantText("(Stopped after $MAX_TOOL_ROUNDS tool-use rounds — try rephrasing.)")
        return Result.success(Unit)
    }

    /**
     * Map the UI-side model id to the Anthropic API model identifier.
     *
     * The picker still lists legacy "claude-3.5-*" labels for continuity, but
     * the API only accepts current ids. Anything non-Anthropic is rejected
     * upstream by the API-key check before we get here.
     */
    private fun resolveModelId(uiId: String): String = when (uiId) {
        "claude-3.5-sonnet" -> "claude-sonnet-4-5"
        "claude-3.5-haiku" -> "claude-haiku-4-5"
        else -> uiId
    }
}
