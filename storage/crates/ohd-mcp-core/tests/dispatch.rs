//! Round-trip tests for the dispatch surface. Each tool gets its own
//! `#[test]` block exercised against an in-memory storage handle.

use ohd_mcp_core::{catalog, dispatch_json};
use ohd_storage_core::{Storage, StorageConfig};
use std::path::PathBuf;

fn open_test_storage() -> Storage {
    // Tempdir keeps the test hermetic and matches how the rest of the
    // workspace tests Storage.
    let dir = tempfile::tempdir().expect("tempdir");
    let path: PathBuf = dir.path().join("test.ohd");
    // Leak the TempDir so the file lives for the duration of the test.
    std::mem::forget(dir);
    Storage::open(StorageConfig::new(path)).expect("open storage")
}

#[test]
fn catalog_contains_phase1_tools() {
    let names: Vec<String> = catalog().into_iter().map(|t| t.name).collect();
    for required in [
        "now",
        "query_events",
        "query_latest",
        "describe_data",
        "summarize",
        "correlate",
        "chart",
        "get_food_log",
        "get_medications_taken",
    ] {
        assert!(names.contains(&required.to_string()), "{required} should be in catalog");
    }
}

#[test]
fn describe_data_on_empty_storage_returns_zero_total() {
    let storage = open_test_storage();
    let out = dispatch_json("describe_data", "{}", &storage);
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert_eq!(v["total_events"].as_i64(), Some(0));
    assert!(v["event_types"].is_array());
}

#[test]
fn query_events_with_no_data_returns_empty_array() {
    let storage = open_test_storage();
    let out = dispatch_json("query_events", "{}", &storage);
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert_eq!(v["count"].as_i64(), Some(0));
    assert!(v["events"].as_array().unwrap().is_empty());
}

#[test]
fn query_events_rejects_event_type_and_prefix_together() {
    let storage = open_test_storage();
    let out = dispatch_json(
        "query_events",
        r#"{"event_type":"food.eaten","event_type_prefix":"intake."}"#,
        &storage,
    );
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert!(v["error"].as_str().unwrap().contains("event_type OR event_type_prefix"));
}

#[test]
fn dispatch_now_returns_iso_and_ms() {
    let storage = open_test_storage();
    let out = dispatch_json("now", "{}", &storage);
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert!(v["iso"].as_str().is_some(), "iso field present");
    assert!(v["timestamp_ms"].as_i64().is_some(), "timestamp_ms field present");
}

#[test]
fn log_symptom_writes_an_event_and_query_finds_it() {
    let storage = open_test_storage();
    let out = dispatch_json(
        "log_symptom",
        r#"{"symptom":"headache","severity":7,"severity_label":"moderate","notes":"after lunch"}"#,
        &storage,
    );
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert_eq!(v["ok"].as_bool(), Some(true), "log_symptom should commit: {v}");
    let written_ulid = v["ulid"].as_str().expect("ulid present").to_string();

    let out = dispatch_json("describe_data", "{}", &storage);
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert!(v["total_events"].as_i64().unwrap_or(0) >= 1);
    let names: Vec<&str> = v["event_types"].as_array().unwrap().iter()
        .filter_map(|t| t["event_type"].as_str()).collect();
    assert!(names.contains(&"symptom.headache"));

    let out = dispatch_json(
        "query_events",
        r#"{"event_type":"symptom.headache","visibility":"all"}"#,
        &storage,
    );
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    let events = v["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["ulid"].as_str(), Some(written_ulid.as_str()));
    let channels = events[0]["channels"].as_object().unwrap();
    assert_eq!(channels.get("severity").and_then(|v| v.as_f64()), Some(7.0));
    assert_eq!(channels.get("severity_label").and_then(|v| v.as_str()), Some("moderate"));
}

#[test]
fn log_food_writes_food_eaten() {
    let storage = open_test_storage();
    let out = dispatch_json(
        "log_food",
        r#"{"description":"oatmeal with banana","grams":250,"notes":"breakfast"}"#,
        &storage,
    );
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert_eq!(v["ok"].as_bool(), Some(true));
    assert_eq!(v["event_type"].as_str(), Some("food.eaten"));
}

#[test]
fn catalog_contains_phase2_log_tools() {
    let names: Vec<String> = catalog().into_iter().map(|t| t.name).collect();
    for required in [
        "log_symptom", "log_food", "log_medication", "log_measurement",
        "log_exercise", "log_mood", "log_sleep", "log_free_event",
    ] {
        assert!(names.contains(&required.to_string()), "{required} should be in catalog");
    }
}

#[test]
fn catalog_contains_phase3_operator_tools() {
    let names: Vec<String> = catalog().into_iter().map(|t| t.name).collect();
    for required in [
        "list_grants", "revoke_grant", "list_pending", "approve_pending",
        "reject_pending", "list_cases", "get_case", "force_close_case",
        "create_grant", "issue_retrospective_grant", "audit_query",
    ] {
        assert!(names.contains(&required.to_string()), "{required} should be in catalog");
    }
}

#[test]
fn full_catalog_count() {
    // Base 28 (1 utility, 8 read, 8 write, 11 operator) + 11 persistent-fact
    // tools (4 read: list_allergies / list_conditions /
    // list_emergency_contacts / get_health_profile; 7 write: record/remove
    // allergy, record/resolve condition, set_blood_type, record/remove
    // emergency_contact) = 39.
    assert_eq!(catalog().len(), 39);
}

#[test]
fn list_grants_on_empty_storage_returns_zero() {
    let storage = open_test_storage();
    let out = dispatch_json("list_grants", "{}", &storage);
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert_eq!(v["count"].as_i64(), Some(0));
}

#[test]
fn audit_query_returns_array() {
    let storage = open_test_storage();
    let out = dispatch_json("audit_query", "{}", &storage);
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert!(v["entries"].is_array());
}

#[test]
fn dispatch_unknown_tool_returns_error_json() {
    let storage = open_test_storage();
    let out = dispatch_json("not_a_tool", "{}", &storage);
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert!(v["error"].as_str().unwrap().contains("unknown tool"));
}

// ---------------------------------------------------------------------------
// Persistent facts (plan deep-dancing-teacup.md)
// ---------------------------------------------------------------------------

#[test]
fn allergy_record_list_remove_round_trip() {
    let storage = open_test_storage();

    // record → appears in list
    let rec = dispatch_json("record_allergy", r#"{"allergen":"Penicillin","severity":"severe","reaction":"hives"}"#, &storage);
    let v: serde_json::Value = serde_json::from_str(&rec).unwrap();
    assert_eq!(v["ok"], true, "record_allergy ok: {rec}");

    let listed = dispatch_json("list_allergies", "{}", &storage);
    let v: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(v["count"], 1, "one allergy listed: {listed}");
    assert_eq!(v["allergies"][0]["allergen"], "Penicillin");
    assert_eq!(v["allergies"][0]["severity"], "severe");
    assert_eq!(v["allergies"][0]["fact_id"], "penicillin");

    // re-record same allergen (updates in place, still one)
    dispatch_json("record_allergy", r#"{"allergen":"Penicillin","severity":"moderate"}"#, &storage);
    let v: serde_json::Value = serde_json::from_str(&dispatch_json("list_allergies", "{}", &storage)).unwrap();
    assert_eq!(v["count"], 1, "still one after update");
    assert_eq!(v["allergies"][0]["severity"], "moderate", "latest wins");

    // remove → gone from list
    dispatch_json("remove_allergy", r#"{"allergen":"Penicillin"}"#, &storage);
    let v: serde_json::Value = serde_json::from_str(&dispatch_json("list_allergies", "{}", &storage)).unwrap();
    assert_eq!(v["count"], 0, "removed allergy gone");
}

#[test]
fn condition_record_resolve_round_trip() {
    let storage = open_test_storage();
    dispatch_json("record_condition", r#"{"name":"Type 2 Diabetes","icd10":"E11"}"#, &storage);
    let v: serde_json::Value = serde_json::from_str(&dispatch_json("list_conditions", "{}", &storage)).unwrap();
    assert_eq!(v["count"], 1);
    assert_eq!(v["conditions"][0]["icd10"], "E11");
    dispatch_json("resolve_condition", r#"{"name":"Type 2 Diabetes"}"#, &storage);
    let v: serde_json::Value = serde_json::from_str(&dispatch_json("list_conditions", "{}", &storage)).unwrap();
    assert_eq!(v["count"], 0, "resolved condition omitted");
}

#[test]
fn blood_type_singleton_latest_wins() {
    let storage = open_test_storage();
    dispatch_json("set_blood_type", r#"{"group":"A","rh":"positive"}"#, &storage);
    dispatch_json("set_blood_type", r#"{"group":"O","rh":"negative"}"#, &storage);
    let v: serde_json::Value = serde_json::from_str(&dispatch_json("get_health_profile", "{}", &storage)).unwrap();
    assert_eq!(v["blood_type"]["group"], "O", "latest blood type wins: {v}");
    assert_eq!(v["blood_type"]["rh"], "negative");
}

#[test]
fn health_profile_bundles_everything() {
    let storage = open_test_storage();
    dispatch_json("record_allergy", r#"{"allergen":"peanuts"}"#, &storage);
    dispatch_json("record_condition", r#"{"name":"asthma"}"#, &storage);
    dispatch_json("set_blood_type", r#"{"group":"AB","rh":"positive"}"#, &storage);
    dispatch_json("record_emergency_contact", r#"{"name":"Jane","relation":"spouse","phone":"+420123"}"#, &storage);
    let v: serde_json::Value = serde_json::from_str(&dispatch_json("get_health_profile", "{}", &storage)).unwrap();
    assert_eq!(v["allergies"].as_array().unwrap().len(), 1);
    assert_eq!(v["conditions"].as_array().unwrap().len(), 1);
    assert_eq!(v["blood_type"]["group"], "AB");
    assert_eq!(v["emergency_contacts"].as_array().unwrap().len(), 1);
    assert_eq!(v["emergency_contacts"][0]["name"], "Jane");
}
