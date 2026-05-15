package com.ohd.connect.data

import android.content.Context
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject

/**
 * Thin passthrough — every tool lives in Rust now (`ohd-mcp-core` crate).
 *
 * The Anthropic-shaped catalog AND each tool's dispatch are exposed
 * through `OhdStorage.list_tools()` + `OhdStorage.execute_tool(name, json)`
 * via uniffi. The phone-side CORD and the standalone MCP server (when it
 * lands) share the same Rust implementation — no Kotlin tool drift.
 */
object CordTools {

    private const val TAG = "OhdConnect.CordTools"

    /**
     * Fetch the live catalog from Rust. Each row maps 1:1 onto
     * [AnthropicClient.Tool] — the Rust crate emits `{name, description,
     * input_schema}` so we just rehydrate.
     */
    fun tools(ctx: Context): List<AnthropicClient.Tool> {
        val raw = StorageRepository.listToolsJson().getOrElse { e ->
            Log.w(TAG, "list_tools failed; returning empty catalog", e)
            return emptyList()
        }
        return parseToolList(raw)
    }

    /**
     * Execute a tool by name. JSON in, JSON out, end-to-end through Rust.
     * Errors come back as `{"error": "..."}` — never throws.
     */
    suspend fun execute(ctx: Context, name: String, input: JSONObject): String {
        return StorageRepository.executeToolJson(name, input.toString())
            .getOrElse { e ->
                Log.w(TAG, "execute_tool($name) failed", e)
                JSONObject().put("error", "tool failed: ${e.message}").toString()
            }
    }

    private fun parseToolList(raw: String): List<AnthropicClient.Tool> = runCatching {
        val arr = JSONArray(raw)
        (0 until arr.length()).map { i ->
            val row = arr.getJSONObject(i)
            AnthropicClient.Tool(
                name = row.getString("name"),
                description = row.getString("description"),
                inputSchema = row.getJSONObject("input_schema"),
            )
        }
    }.getOrElse {
        Log.w(TAG, "couldn't parse list_tools response: $raw", it)
        emptyList()
    }
}
