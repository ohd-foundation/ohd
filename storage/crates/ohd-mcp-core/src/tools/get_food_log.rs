//! `get_food_log` — last week of food.eaten events, optionally with
//! nutrition aggregates over the window.

use crate::event_json::{event_to_json, ms_to_iso, now_ms, parse_iso, scalar_numeric};
use crate::ToolResult;
use ohd_storage_core::events::{query_events, EventFilter, EventVisibility};
use ohd_storage_core::Storage;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const NAME: &str = "get_food_log";

pub const DESCRIPTION: &str =
    "Return the user's food log (food.eaten events). Defaults to the last 7 days. When \
     `include_nutrition_totals = true` (default), also returns aggregate intake.* totals \
     for the same window — kcal, carbs, protein, fat, sugar, fiber, caffeine, etc.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "from_iso":                 { "type": "string" },
            "to_iso":                   { "type": "string" },
            "include_nutrition_totals": { "type": "boolean", "default": true }
        },
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let to_ms = input.get("to_iso").and_then(|v| v.as_str()).and_then(parse_iso).unwrap_or_else(now_ms);
    let from_ms = input.get("from_iso").and_then(|v| v.as_str()).and_then(parse_iso)
        .unwrap_or_else(|| to_ms - 7 * 86_400_000);
    let include_totals = input.get("include_nutrition_totals").and_then(|v| v.as_bool()).unwrap_or(true);

    let (meals, _) = storage.with_conn(|conn| query_events(conn, &EventFilter {
        from_ms: Some(from_ms),
        to_ms: Some(to_ms),
        event_types_in: vec!["food.eaten".into()],
        limit: Some(500),
        ..Default::default()
    }, None))?;
    let meals_json: Vec<Value> = meals.iter().map(event_to_json).collect();

    let totals = if include_totals {
        // Sum each intake.* child event's `value` channel, grouped by event_type.
        let prefix = "intake.";
        let intake_types: Vec<String> = storage.with_conn(|conn| {
            // Include `custom.intake.*` shadows so unpromoted intake.* types
            // still get summed into the food log totals.
            let mut stmt = conn.prepare(
                "SELECT namespace, name FROM event_types
                 WHERE namespace || '.' || name LIKE ?1 || '%'
                    OR (namespace = 'custom' AND name LIKE ?1 || '%')",
            )?;
            let rows: Result<Vec<String>, ohd_storage_core::Error> = stmt
                .query_map([prefix], |r| Ok(format!("{}.{}", r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .map(|r| r.map_err(ohd_storage_core::Error::from))
                .collect();
            rows
        })?;
        let (intake, _) = storage.with_conn(|conn| query_events(conn, &EventFilter {
            from_ms: Some(from_ms),
            to_ms: Some(to_ms),
            event_types_in: intake_types,
            limit: Some(20_000),
            visibility: EventVisibility::All,
            ..Default::default()
        }, None))?;
        let mut sums: HashMap<String, (f64, i64)> = HashMap::new();
        for e in &intake {
            let v = e.channels.iter().find(|c| c.channel_path == "value").and_then(|c| scalar_numeric(&c.value));
            if let Some(v) = v {
                let entry = sums.entry(e.event_type.clone()).or_insert((0.0, 0));
                entry.0 += v;
                entry.1 += 1;
            }
        }
        let mut map = serde_json::Map::new();
        for (k, (sum, n)) in sums {
            map.insert(k, json!({ "sum": sum, "n": n }));
        }
        Some(Value::Object(map))
    } else {
        None
    };

    Ok(json!({
        "from_iso": ms_to_iso(from_ms),
        "to_iso": ms_to_iso(to_ms),
        "meal_count": meals_json.len(),
        "meals": meals_json,
        "nutrition_totals": totals,
    }))
}
