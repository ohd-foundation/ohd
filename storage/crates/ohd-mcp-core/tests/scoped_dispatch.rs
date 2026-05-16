//! Integration tests for share-scope enforcement on the dispatch surface.
//!
//! Each test opens an in-memory storage, writes a couple of events, mints
//! a grant with specific rules, builds a `ShareScope` from it, and drives
//! `dispatch_scoped_json` — verifying the phone-side enforcement boundary
//! described in `cord/spec/data-link.md` "Scope enforcement".

use ohd_mcp_core::{
    catalog, catalog_scoped, dispatch_json, dispatch_scoped_json, ShareScope,
};
use ohd_storage_core::grants::{create_grant, read_grant, set_grant_suspended, NewGrant, RuleEffect};
use ohd_storage_core::{Storage, StorageConfig};
use std::path::PathBuf;

fn open_test_storage() -> Storage {
    let dir = tempfile::tempdir().expect("tempdir");
    let path: PathBuf = dir.path().join("test.ohd");
    std::mem::forget(dir);
    Storage::open(StorageConfig::new(path)).expect("open storage")
}

/// Mint a grant from a sparse `NewGrant`, returning its row id.
fn mint_grant(storage: &Storage, g: NewGrant) -> i64 {
    storage
        .with_conn_mut(|conn| create_grant(conn, &g).map(|(id, _)| id))
        .expect("create grant")
}

fn scope_for(storage: &Storage, grant_id: i64) -> ShareScope {
    let row = storage
        .with_conn(|conn| read_grant(conn, grant_id))
        .expect("read grant");
    ShareScope::from_grant(&row, ohd_mcp_core::event_json::now_ms())
}

fn base_grant() -> NewGrant {
    NewGrant {
        grantee_label: "CORD".into(),
        grantee_kind: "service".into(),
        approval_mode: "never_required".into(),
        default_action: RuleEffect::Deny,
        ..Default::default()
    }
}

#[test]
fn in_scope_query_passes_out_of_scope_type_denied() {
    let storage = open_test_storage();
    // Two events of different types.
    dispatch_json(
        "log_symptom",
        r#"{"symptom":"headache","severity":5}"#,
        &storage,
    );
    dispatch_json(
        "log_food",
        r#"{"description":"oatmeal","grams":200}"#,
        &storage,
    );

    // Grant allows reading food.eaten only.
    let g = NewGrant {
        event_type_rules: vec![("food.eaten".into(), RuleEffect::Allow)],
        ..base_grant()
    };
    let grant_id = mint_grant(&storage, g);
    let scope = scope_for(&storage, grant_id);

    // In-scope query succeeds and returns the food event.
    let out = dispatch_scoped_json(
        "query_events",
        r#"{"event_type":"food.eaten","visibility":"all"}"#,
        &storage,
        Some(&scope),
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["error"].is_null(), "in-scope query must not error: {v}");
    assert_eq!(v["count"].as_i64(), Some(1));

    // Out-of-scope explicit type → "not permitted", never empty data.
    let out = dispatch_scoped_json(
        "query_events",
        r#"{"event_type":"symptom.headache","visibility":"all"}"#,
        &storage,
        Some(&scope),
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let err = v["error"].as_str().expect("error present");
    assert!(err.contains("not permitted"), "expected not-permitted: {err}");
}

#[test]
fn out_of_scope_rows_filtered_from_unfiltered_query() {
    let storage = open_test_storage();
    dispatch_json("log_symptom", r#"{"symptom":"headache"}"#, &storage);
    dispatch_json("log_food", r#"{"description":"toast"}"#, &storage);

    let g = NewGrant {
        event_type_rules: vec![("food.eaten".into(), RuleEffect::Allow)],
        ..base_grant()
    };
    let scope = scope_for(&storage, mint_grant(&storage, g));

    // A query with no event_type would normally return both rows; the
    // scope must drop the symptom row.
    let out = dispatch_scoped_json(
        "query_events",
        r#"{"visibility":"all"}"#,
        &storage,
        Some(&scope),
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let events = v["events"].as_array().unwrap();
    assert_eq!(events.len(), 1, "only the in-scope row survives");
    assert_eq!(events[0]["event_type"], "food.eaten");
    assert_eq!(v["count"].as_i64(), Some(1));
}

#[test]
fn write_tool_hidden_and_rejected_for_read_only_scope() {
    let storage = open_test_storage();
    let g = NewGrant {
        default_action: RuleEffect::Allow,
        ..base_grant()
    };
    let scope = scope_for(&storage, mint_grant(&storage, g));

    // Catalog omits write tools but keeps read tools.
    let names: Vec<String> = catalog_scoped(Some(&scope))
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(names.contains(&"query_events".to_string()));
    assert!(!names.contains(&"log_food".to_string()), "write tool hidden");
    assert!(names.len() < catalog().len());

    // Dispatching a write tool is rejected as not-permitted.
    let out = dispatch_scoped_json(
        "log_food",
        r#"{"description":"banana"}"#,
        &storage,
        Some(&scope),
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["error"].as_str().unwrap().contains("not permitted"));
}

#[test]
fn write_scope_permits_its_event_type() {
    let storage = open_test_storage();
    let g = NewGrant {
        default_action: RuleEffect::Allow,
        write_event_type_rules: vec![("food.eaten".into(), RuleEffect::Allow)],
        ..base_grant()
    };
    let scope = scope_for(&storage, mint_grant(&storage, g));

    let names: Vec<String> = catalog_scoped(Some(&scope))
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert!(names.contains(&"log_food".to_string()), "write tool listed");

    let out = dispatch_scoped_json(
        "log_food",
        r#"{"description":"apple"}"#,
        &storage,
        Some(&scope),
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["ok"].as_bool(), Some(true), "in-scope write commits: {v}");

    // A write of a type the grant doesn't carry is rejected.
    let out = dispatch_scoped_json(
        "log_mood",
        r#"{"mood":"good"}"#,
        &storage,
        Some(&scope),
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["error"].as_str().unwrap().contains("not permitted"));
}

#[test]
fn operator_tools_are_never_exposed_to_a_share() {
    let storage = open_test_storage();
    let g = NewGrant {
        default_action: RuleEffect::Allow,
        write_event_type_rules: vec![("food.eaten".into(), RuleEffect::Allow)],
        ..base_grant()
    };
    let scope = scope_for(&storage, mint_grant(&storage, g));
    let names: Vec<String> = catalog_scoped(Some(&scope))
        .into_iter()
        .map(|t| t.name)
        .collect();
    for op in ["list_grants", "create_grant", "revoke_grant", "audit_query"] {
        assert!(!names.contains(&op.to_string()), "{op} must not be exposed");
    }
    let out = dispatch_scoped_json("list_grants", "{}", &storage, Some(&scope));
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v["error"].as_str().unwrap().contains("not permitted"));
}

#[test]
fn suspended_scope_denies_all_reads() {
    let storage = open_test_storage();
    dispatch_json("log_food", r#"{"description":"rice"}"#, &storage);
    let g = NewGrant {
        default_action: RuleEffect::Allow,
        ..base_grant()
    };
    let grant_id = mint_grant(&storage, g);
    storage
        .with_conn(|conn| set_grant_suspended(conn, grant_id, true))
        .expect("suspend");
    let scope = scope_for(&storage, grant_id);
    assert!(scope.is_denied());

    let out = dispatch_scoped_json(
        "query_events",
        r#"{"event_type":"food.eaten","visibility":"all"}"#,
        &storage,
        Some(&scope),
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(
        v["error"].as_str().unwrap().contains("suspended"),
        "suspended share denies reads: {v}"
    );
}

#[test]
fn none_scope_is_unchanged_owner_behaviour() {
    let storage = open_test_storage();
    dispatch_json("log_food", r#"{"description":"pasta"}"#, &storage);
    // None scope == the unscoped path.
    let scoped = dispatch_scoped_json("query_events", "{}", &storage, None);
    let plain = dispatch_json("query_events", "{}", &storage);
    let a: serde_json::Value = serde_json::from_str(&scoped).unwrap();
    let b: serde_json::Value = serde_json::from_str(&plain).unwrap();
    assert_eq!(a["count"], b["count"]);
    assert_eq!(catalog_scoped(None).len(), catalog().len());
}
