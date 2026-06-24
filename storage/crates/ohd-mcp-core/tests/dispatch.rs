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
    // Base 28 (1 utility, 8 read, 8 write, 11 operator)
    // + 11 persistent-fact tools (4 read, 7 write)
    // + 3 medication-regimen tools (1 read, 2 write)
    // + 6 case/clinical tools (1 read get_case_timeline, 3 write
    //   record_doctor_visit/prescription/lab_result, 2 operator
    //   open_case/close_case)
    // + 4 watch/treatment-plan tools (2 read list_measurement_watches/
    //   get_treatment_plan, 2 write start/stop_measurement_watch)
    // + 1 operator delete_event = 53.
    assert_eq!(catalog().len(), 53);
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

// ---------------------------------------------------------------------------
// Medication regimens (plan deep-dancing-teacup.md, phase 3)
// ---------------------------------------------------------------------------

#[test]
fn regimen_start_list_discontinue_round_trip() {
    let storage = open_test_storage();

    let started = dispatch_json(
        "start_medication_regimen",
        r#"{"name":"Metformin","dose_value":500,"dose_unit":"mg","frequency":"twice daily"}"#,
        &storage,
    );
    let v: serde_json::Value = serde_json::from_str(&started).unwrap();
    assert_eq!(v["ok"], true, "start ok: {started}");
    let regimen_id = v["regimen_id"].as_str().expect("regimen_id returned").to_string();
    assert!(!regimen_id.is_empty());

    let listed = dispatch_json("list_active_regimens", "{}", &storage);
    let v: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(v["count"], 1, "one active regimen: {listed}");
    assert_eq!(v["regimens"][0]["name"], "Metformin");
    assert_eq!(v["regimens"][0]["dose_value"], 500.0);
    assert_eq!(v["regimens"][0]["regimen_id"], regimen_id);

    // a dose linked to the regimen, recording the ACTUAL dose taken
    let dose = dispatch_json(
        "log_medication",
        &format!(r#"{{"name":"Metformin","regimen_id":"{regimen_id}","dose_value":250,"dose_unit":"mg","status":"taken","dose_note":"half — felt nauseous"}}"#),
        &storage,
    );
    let v: serde_json::Value = serde_json::from_str(&dose).unwrap();
    assert_eq!(v["ok"], true, "dose logged: {dose}");

    // discontinue → no longer active
    let stop = dispatch_json(
        "discontinue_medication_regimen",
        &format!(r#"{{"regimen_id":"{regimen_id}","reason":"course finished"}}"#),
        &storage,
    );
    let v: serde_json::Value = serde_json::from_str(&stop).unwrap();
    assert_eq!(v["ok"], true, "discontinue ok: {stop}");
    let v: serde_json::Value = serde_json::from_str(&dispatch_json("list_active_regimens", "{}", &storage)).unwrap();
    assert_eq!(v["count"], 0, "discontinued regimen gone");
}

#[test]
fn skipped_dose_records_skipped_bool() {
    let storage = open_test_storage();
    dispatch_json("log_medication", r#"{"name":"aspirin","status":"skipped","adherence_reason":"forgot"}"#, &storage);
    let v: serde_json::Value = serde_json::from_str(
        &dispatch_json("query_events", r#"{"event_type":"medication.taken","visibility":"all"}"#, &storage),
    ).unwrap();
    let ch = &v["events"][0]["channels"];
    assert_eq!(ch["status"], "skipped");
    assert_eq!(ch["skipped"], true, "skipped bool channel present: {v}");
}

#[test]
fn health_profile_includes_active_regimens() {
    let storage = open_test_storage();
    dispatch_json("start_medication_regimen", r#"{"name":"Lisinopril","dose_value":10,"dose_unit":"mg"}"#, &storage);
    let v: serde_json::Value = serde_json::from_str(&dispatch_json("get_health_profile", "{}", &storage)).unwrap();
    assert_eq!(v["medications"].as_array().unwrap().len(), 1, "regimen in health profile: {v}");
    assert_eq!(v["medications"][0]["name"], "Lisinopril");
}

// ---------------------------------------------------------------------------
// Clinical cases + events (plan deep-dancing-teacup.md, phase 4)
// ---------------------------------------------------------------------------

#[test]
fn doctor_visit_prescription_timeline_round_trip() {
    let storage = open_test_storage();

    // record a visit → opens a case
    let visit = dispatch_json(
        "record_doctor_visit",
        r#"{"practitioner_name":"Dr. Novak","specialty":"gastroenterology","reason":"stomach pain"}"#,
        &storage,
    );
    let v: serde_json::Value = serde_json::from_str(&visit).unwrap();
    assert_eq!(v["ok"], true, "visit recorded: {visit}");
    let case_ulid = v["case_ulid"].as_str().expect("case_ulid").to_string();

    // prescribe within the case → also starts a regimen
    let rx = dispatch_json(
        "record_prescription",
        &format!(r#"{{"case_id":"{case_ulid}","medication_name":"Metformin","dose_value":500,"dose_unit":"mg","frequency":"twice daily"}}"#),
        &storage,
    );
    let v: serde_json::Value = serde_json::from_str(&rx).unwrap();
    assert_eq!(v["ok"], true, "prescription recorded: {rx}");

    // the prescribed drug now shows as an active regimen
    let v: serde_json::Value = serde_json::from_str(&dispatch_json("list_active_regimens", "{}", &storage)).unwrap();
    assert_eq!(v["count"], 1, "prescription started a regimen: {v}");
    assert_eq!(v["regimens"][0]["name"], "Metformin");
    assert_eq!(v["regimens"][0]["case_id"], case_ulid);

    // a lab result in the same case
    dispatch_json(
        "record_lab_result",
        &format!(r#"{{"case_id":"{case_ulid}","test_name":"HbA1c","value":7.2,"unit":"%","reference_range":"4-5.6"}}"#),
        &storage,
    );

    // the case timeline returns everything tagged to the case: the visit,
    // the prescription, the lab result, AND the regimen the prescription
    // started (it carries case_id for provenance — which visit put the
    // user on this drug).
    let tl = dispatch_json("get_case_timeline", &format!(r#"{{"case_ulid":"{case_ulid}"}}"#), &storage);
    let v: serde_json::Value = serde_json::from_str(&tl).unwrap();
    assert_eq!(v["count"], 4, "visit + prescription + regimen + lab tagged to the case: {tl}");
    let types: Vec<&str> = v["events"].as_array().unwrap().iter()
        .filter_map(|e| e["event_type"].as_str()).collect();
    assert!(types.contains(&"clinical.visit"));
    assert!(types.contains(&"clinical.prescription"));
    assert!(types.contains(&"clinical.lab_result"));
    assert!(types.contains(&"medication.regimen_started"));

    // close the case
    let close = dispatch_json("close_case", &format!(r#"{{"case_ulid":"{case_ulid}"}}"#), &storage);
    let v: serde_json::Value = serde_json::from_str(&close).unwrap();
    assert_eq!(v["ok"], true, "case closed: {close}");
}

#[test]
fn delete_event_removes_by_ulid() {
    let storage = open_test_storage();
    // Log a dose, grab its ULID, delete it, confirm it's gone.
    let logged = dispatch_json("log_medication", r#"{"name":"aspirin","status":"taken"}"#, &storage);
    let ulid = serde_json::from_str::<serde_json::Value>(&logged).unwrap()["ulid"]
        .as_str().expect("ulid").to_string();

    let before: serde_json::Value = serde_json::from_str(
        &dispatch_json("query_events", r#"{"event_type":"medication.taken","visibility":"all"}"#, &storage),
    ).unwrap();
    assert_eq!(before["events"].as_array().unwrap().len(), 1);

    let del = dispatch_json("delete_event", &format!(r#"{{"ulid":"{ulid}"}}"#), &storage);
    let v: serde_json::Value = serde_json::from_str(&del).unwrap();
    assert_eq!(v["ok"], true, "delete ok: {del}");
    assert_eq!(v["deleted"], 1, "one row deleted: {del}");

    let after: serde_json::Value = serde_json::from_str(
        &dispatch_json("query_events", r#"{"event_type":"medication.taken","visibility":"all"}"#, &storage),
    ).unwrap();
    assert_eq!(after["events"].as_array().unwrap().len(), 0, "event gone after delete");
}

#[test]
fn open_case_returns_ulid() {
    let storage = open_test_storage();
    let out = dispatch_json("open_case", r#"{"case_type":"illness","label":"flu"}"#, &storage);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["ok"], true, "{out}");
    assert!(v["case_ulid"].as_str().is_some_and(|s| !s.is_empty()));
}

#[test]
fn measurement_watch_start_list_stop_round_trip() {
    let storage = open_test_storage();

    let started = dispatch_json(
        "start_measurement_watch",
        r#"{"metric":"body_temperature","label":"Temp","schedule":"0 8 * * *","quick":true}"#,
        &storage,
    );
    let v: serde_json::Value = serde_json::from_str(&started).unwrap();
    assert_eq!(v["ok"], true, "watch start ok: {started}");
    let watch_id = v["watch_id"].as_str().expect("watch_id returned").to_string();
    assert!(!watch_id.is_empty());

    let listed = dispatch_json("list_measurement_watches", "{}", &storage);
    let v: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(v["count"], 1, "one active watch: {listed}");
    assert_eq!(v["watches"][0]["metric"], "body_temperature");
    assert_eq!(v["watches"][0]["schedule"], "0 8 * * *");
    assert_eq!(v["watches"][0]["quick"], true);
    assert_eq!(v["watches"][0]["watch_id"], watch_id);

    let stop = dispatch_json(
        "stop_measurement_watch",
        &format!(r#"{{"watch_id":"{watch_id}","reason":"recovered"}}"#),
        &storage,
    );
    let v: serde_json::Value = serde_json::from_str(&stop).unwrap();
    assert_eq!(v["ok"], true, "watch stop ok: {stop}");
    let v: serde_json::Value =
        serde_json::from_str(&dispatch_json("list_measurement_watches", "{}", &storage)).unwrap();
    assert_eq!(v["count"], 0, "stopped watch gone");
}

#[test]
fn treatment_plan_splits_case_vs_life() {
    let storage = open_test_storage();

    // A personal med (no case) and a watch on a specific case.
    dispatch_json(
        "start_medication_regimen",
        r#"{"name":"Vitamin D","dose_value":1000,"dose_unit":"IU","on_hand":true,"quick":true}"#,
        &storage,
    );
    let case = dispatch_json("open_case", r#"{"case_type":"illness","label":"flu"}"#, &storage);
    let case_ulid = serde_json::from_str::<serde_json::Value>(&case).unwrap()["case_ulid"]
        .as_str()
        .unwrap()
        .to_string();
    dispatch_json(
        "start_measurement_watch",
        &format!(r#"{{"metric":"body_temperature","case_id":"{case_ulid}","schedule":"0 */8 * * *"}}"#),
        &storage,
    );

    // The case plan: the watch, not the personal vitamin.
    let plan = dispatch_json(
        "get_treatment_plan",
        &format!(r#"{{"case_id":"{case_ulid}"}}"#),
        &storage,
    );
    let v: serde_json::Value = serde_json::from_str(&plan).unwrap();
    assert_eq!(v["medications"].as_array().unwrap().len(), 0, "no case meds: {plan}");
    assert_eq!(v["watches"].as_array().unwrap().len(), 1, "case watch present: {plan}");

    // The global "life" plan (no case_id): the vitamin, not the case watch.
    let life = dispatch_json("get_treatment_plan", "{}", &storage);
    let v: serde_json::Value = serde_json::from_str(&life).unwrap();
    assert_eq!(v["medications"].as_array().unwrap().len(), 1, "personal med present: {life}");
    assert_eq!(v["medications"][0]["name"], "Vitamin D");
    assert_eq!(v["watches"].as_array().unwrap().len(), 0, "no life watch: {life}");
}
