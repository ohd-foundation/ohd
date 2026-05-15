package com.ohd.connect.data

import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runInterruptible
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/**
 * Thin wrapper around `POST https://api.anthropic.com/v1/messages`.
 *
 * Synchronous JSON (no SSE streaming) — keeps the call shape symmetric with
 * the other HTTP clients in this package and lets [CordRunner] iterate the
 * tool-use loop without juggling stream state.
 *
 * Prompt caching is requested on the (system + tools) prefix by stamping
 * `"cache_control": { "type": "ephemeral" }` on the last system block; once
 * the cache is warm subsequent turns in the same chat skip re-billing the
 * shared prefix.
 */
object AnthropicClient {

    private const val TAG = "OhdConnect.Anthropic"

    private const val ENDPOINT = "https://api.anthropic.com/v1/messages"
    private const val MODELS_ENDPOINT = "https://api.anthropic.com/v1/models"
    private const val ANTHROPIC_VERSION = "2023-06-01"
    private const val CONNECT_TIMEOUT_MS = 10_000
    private const val READ_TIMEOUT_MS = 60_000

    /** One turn in the conversation — either user or assistant. */
    data class Msg(val role: String, val content: List<ContentBlock>)

    sealed interface ContentBlock {
        data class Text(val text: String) : ContentBlock
        data class ToolUse(val id: String, val name: String, val input: JSONObject) : ContentBlock
        data class ToolResult(
            val toolUseId: String,
            val content: String,
            val isError: Boolean = false,
        ) : ContentBlock
    }

    data class Tool(
        val name: String,
        val description: String,
        val inputSchema: JSONObject,
    )

    /** Parsed `/v1/messages` response — only the bits [CordRunner] needs. */
    data class MsgResponse(
        val content: List<ContentBlock>,
        val stopReason: String,
    )

    data class ModelInfo(
        val id: String,
        val displayName: String,
        val createdAt: String?,
    )

    /**
     * `GET /v1/models` — current model catalog. Used by CORD settings so
     * the picker reflects what the API actually accepts rather than
     * hardcoded ids that go stale every release.
     */
    suspend fun listModels(apiKey: String): Result<List<ModelInfo>> =
        runInterruptible(Dispatchers.IO) {
            runCatching {
                val conn = (URL(MODELS_ENDPOINT).openConnection() as HttpURLConnection).apply {
                    requestMethod = "GET"
                    connectTimeout = CONNECT_TIMEOUT_MS
                    readTimeout = READ_TIMEOUT_MS
                    val isOauth = apiKey.startsWith("sk-ant-oat")
                    if (isOauth) {
                        setRequestProperty("authorization", "Bearer $apiKey")
                        setRequestProperty(
                            "anthropic-beta",
                            "oauth-2025-04-20,claude-code-2025-04-08",
                        )
                    } else {
                        setRequestProperty("x-api-key", apiKey)
                    }
                    setRequestProperty("anthropic-version", ANTHROPIC_VERSION)
                    setRequestProperty("accept", "application/json")
                }
                try {
                    val code = conn.responseCode
                    val stream = if (code in 200..299) conn.inputStream else conn.errorStream
                    val text = stream?.bufferedReader()?.use { it.readText() }.orEmpty()
                    if (code !in 200..299) {
                        error("Anthropic models HTTP $code: ${text.take(400)}")
                    }
                    val arr = JSONObject(text).optJSONArray("data") ?: return@runCatching emptyList()
                    val out = ArrayList<ModelInfo>(arr.length())
                    for (i in 0 until arr.length()) {
                        val row = arr.getJSONObject(i)
                        out += ModelInfo(
                            id = row.getString("id"),
                            displayName = row.optString("display_name", row.getString("id")),
                            createdAt = row.optString("created_at").takeIf { it.isNotEmpty() },
                        )
                    }
                    out
                } finally {
                    conn.disconnect()
                }
            }
        }

    suspend fun messages(
        apiKey: String,
        model: String,
        system: String,
        messages: List<Msg>,
        tools: List<Tool>,
        maxTokens: Int = 1024,
    ): Result<MsgResponse> = runInterruptible(Dispatchers.IO) {
        callWithRetry(apiKey, model, system, messages, tools, maxTokens)
    }

    /**
     * Retry transient failures (429 + 5xx) with exponential backoff. Honours
     * `Retry-After` when the server sends one (Anthropic does on most 429s),
     * otherwise doubles a 500-ms base up to ~8 s. Total cap: 4 attempts.
     */
    private fun callWithRetry(
        apiKey: String,
        model: String,
        system: String,
        messages: List<Msg>,
        tools: List<Tool>,
        maxTokens: Int,
    ): Result<MsgResponse> {
        val maxAttempts = 4
        var lastError: Throwable? = null
        repeat(maxAttempts) { attempt ->
            val outcome = runCatching { call(apiKey, model, system, messages, tools, maxTokens) }
            if (outcome.isSuccess) return outcome
            val err = outcome.exceptionOrNull() ?: error("unreachable")
            lastError = err
            val transient = err is TransientHttpError
            if (!transient || attempt == maxAttempts - 1) return Result.failure(err)
            val retryAfter = (err as TransientHttpError).retryAfterMs
            val backoff = retryAfter ?: (500L * (1 shl attempt)).coerceAtMost(8_000L)
            Log.w(TAG, "Anthropic ${err.status} → retry in ${backoff}ms (attempt ${attempt + 1}/$maxAttempts)")
            Thread.sleep(backoff)
        }
        return Result.failure(lastError ?: IllegalStateException("retry loop exhausted"))
    }

    private class TransientHttpError(
        val status: Int,
        val retryAfterMs: Long?,
        body: String,
    ) : Exception("Anthropic HTTP $status: ${body.take(400)}")

    private fun call(
        apiKey: String,
        model: String,
        system: String,
        messages: List<Msg>,
        tools: List<Tool>,
        maxTokens: Int,
    ): MsgResponse {
        val body = buildRequest(model, system, messages, tools, maxTokens)
        val conn = (URL(ENDPOINT).openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"
            connectTimeout = CONNECT_TIMEOUT_MS
            readTimeout = READ_TIMEOUT_MS
            doOutput = true
            // Two credential shapes:
            //  - "sk-ant-api…"  → standard API key, sent via x-api-key.
            //  - "sk-ant-oat…"  → Claude Code OAuth token, sent as
            //    Authorization: Bearer + the `oauth-2025-04-20` beta flag.
            //    The Claude Code subscription pool gates Sonnet/Opus
            //    behind a `claude-code-2025-04-08` beta flag — without
            //    it OAuth tokens only resolve Haiku. Anything else is
            //    best-effort treated as an API key.
            val isOauth = apiKey.startsWith("sk-ant-oat")
            if (isOauth) {
                setRequestProperty("authorization", "Bearer $apiKey")
                setRequestProperty(
                    "anthropic-beta",
                    "oauth-2025-04-20,claude-code-2025-04-08",
                )
            } else {
                setRequestProperty("x-api-key", apiKey)
            }
            setRequestProperty("anthropic-version", ANTHROPIC_VERSION)
            setRequestProperty("content-type", "application/json")
            setRequestProperty("accept", "application/json")
        }
        try {
            conn.outputStream.use { it.write(body.toString().toByteArray(Charsets.UTF_8)) }
            val code = conn.responseCode
            val stream = if (code in 200..299) conn.inputStream else conn.errorStream
            val text = stream?.bufferedReader()?.use { it.readText() }.orEmpty()
            if (code !in 200..299) {
                Log.w(TAG, "POST /v1/messages → $code")
                if (code == 429 || code in 500..599) {
                    val retryAfterMs = conn.getHeaderField("Retry-After")?.let { raw ->
                        raw.toLongOrNull()?.times(1_000L)
                    }
                    throw TransientHttpError(code, retryAfterMs, text)
                }
                error("Anthropic HTTP $code: ${text.take(400)}")
            }
            return parseResponse(JSONObject(text))
        } finally {
            conn.disconnect()
        }
    }

    private fun buildRequest(
        model: String,
        system: String,
        messages: List<Msg>,
        tools: List<Tool>,
        maxTokens: Int,
    ): JSONObject {
        val root = JSONObject()
        root.put("model", model)
        root.put("max_tokens", maxTokens)
        // System as an array of blocks lets us attach cache_control to the
        // last block so the prefix (system + tools) gets re-used across turns.
        val systemArr = JSONArray().apply {
            put(
                JSONObject()
                    .put("type", "text")
                    .put("text", system)
                    .put("cache_control", JSONObject().put("type", "ephemeral")),
            )
        }
        root.put("system", systemArr)
        root.put("messages", encodeMessages(messages))
        if (tools.isNotEmpty()) {
            root.put("tools", encodeTools(tools))
        }
        return root
    }

    private fun encodeMessages(messages: List<Msg>): JSONArray {
        val arr = JSONArray()
        for (msg in messages) {
            val obj = JSONObject()
            obj.put("role", msg.role)
            val contentArr = JSONArray()
            for (block in msg.content) contentArr.put(encodeBlock(block))
            obj.put("content", contentArr)
            arr.put(obj)
        }
        return arr
    }

    private fun encodeBlock(block: ContentBlock): JSONObject = when (block) {
        is ContentBlock.Text -> JSONObject()
            .put("type", "text")
            .put("text", block.text)

        is ContentBlock.ToolUse -> JSONObject()
            .put("type", "tool_use")
            .put("id", block.id)
            .put("name", block.name)
            .put("input", block.input)

        is ContentBlock.ToolResult -> JSONObject()
            .put("type", "tool_result")
            .put("tool_use_id", block.toolUseId)
            .put("content", block.content)
            .apply { if (block.isError) put("is_error", true) }
    }

    private fun encodeTools(tools: List<Tool>): JSONArray {
        val arr = JSONArray()
        for ((idx, tool) in tools.withIndex()) {
            val obj = JSONObject()
                .put("name", tool.name)
                .put("description", tool.description)
                .put("input_schema", tool.inputSchema)
            // Cache the tail of the tool list so the tools block also enters
            // the cached prefix alongside the system text.
            if (idx == tools.size - 1) {
                obj.put("cache_control", JSONObject().put("type", "ephemeral"))
            }
            arr.put(obj)
        }
        return arr
    }

    private fun parseResponse(root: JSONObject): MsgResponse {
        val stopReason = root.optString("stop_reason", "end_turn")
        val contentArr = root.optJSONArray("content") ?: JSONArray()
        val blocks = ArrayList<ContentBlock>(contentArr.length())
        for (i in 0 until contentArr.length()) {
            val block = contentArr.optJSONObject(i) ?: continue
            when (block.optString("type")) {
                "text" -> blocks += ContentBlock.Text(block.optString("text"))
                "tool_use" -> blocks += ContentBlock.ToolUse(
                    id = block.optString("id"),
                    name = block.optString("name"),
                    input = block.optJSONObject("input") ?: JSONObject(),
                )
            }
        }
        return MsgResponse(content = blocks, stopReason = stopReason)
    }
}
