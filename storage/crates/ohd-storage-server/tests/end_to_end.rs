//! End-to-end OHDC Connect-RPC smoke test.
//!
//! Boots the server in-process against a temp DB, issues a self-session
//! token, drives the OHDC RPCs over **real Connect-RPC framing** (Protobuf
//! binary + Connect-Protocol-Version headers, gRPC-compatible) over HTTP/2
//! (h2c), and asserts the put-then-query round-trip plus the unauthenticated
//! 401 path. The wire is not the previous JSON-over-HTTP/1.1 — `connectrpc
//! 0.4` framing is binary by default, JSON only on explicit negotiation.

use std::sync::Arc;

use connectrpc::client::{ClientConfig, Http2Connection};
use connectrpc::ConnectError;
use ohd_storage_core::{
    auth::{issue_grant_token, issue_self_session_token, TokenKind},
    grants::{create_grant, NewGrant, RuleEffect},
    storage::{Storage, StorageConfig},
    ulid as ohd_ulid,
};

// Pull in the binary's `server.rs` + the codegen-emitted proto module so the
// test exercises the same router & client types the production binary uses.
#[allow(dead_code)]
#[path = "../src/server.rs"]
mod server;

#[allow(dead_code)]
#[path = "../src/sync_server.rs"]
mod sync_server;

#[allow(dead_code)]
#[path = "../src/auth_server.rs"]
mod auth_server;

#[allow(dead_code)]
#[path = "../src/jwks.rs"]
mod jwks;

#[allow(dead_code)]
#[path = "../src/oauth.rs"]
mod oauth;

mod proto {
    connectrpc::include_generated!();
}

use proto::ohdc::v0 as pb;
use proto::ohdc::v0::OhdcServiceClient;

#[tokio::test(flavor = "multi_thread")]
async fn connect_rpc_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("e2e.db");
    let storage = Arc::new(Storage::open(StorageConfig::new(&path)).unwrap());
    let user_ulid = storage.user_ulid();
    let bearer = storage
        .with_conn(|conn| issue_self_session_token(conn, user_ulid, Some("e2e"), None))
        .unwrap();

    // Bind to an ephemeral port and drive the OHDC router under the same
    // `Storage` handle. We use `connectrpc::Server` directly so the wire
    // matches what the binary speaks.
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = std_listener.local_addr().unwrap();
    std_listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();

    let router = server::router(storage.clone());
    let server_handle = tokio::spawn(async move {
        let bound = connectrpc::Server::from_listener(listener);
        bound.serve(router).await.expect("server died");
    });

    // Give the server a beat to start accepting.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // ---- Build a Connect-RPC client over plaintext HTTP/2. ----
    let uri: http::Uri = format!("http://{addr}").parse().unwrap();
    let conn = Http2Connection::connect_plaintext(uri.clone())
        .await
        .expect("h2 connect")
        .shared(64);
    let config = ClientConfig::new(uri.clone())
        .protocol(connectrpc::Protocol::Grpc)
        .default_header("authorization", format!("Bearer {bearer}"));
    let client = OhdcServiceClient::new(conn, config);

    // ---- Health (no auth required, but our default header is harmless) ----
    let resp = client
        .health(pb::HealthRequest::default())
        .await
        .expect("health");
    // Confirm the wire is real Connect-RPC framing. With Protocol::Grpc the
    // response carries `content-type: application/grpc+proto` (binary
    // Protobuf framing) and a `grpc-status` trailer — NOT JSON over
    // HTTP/1.1. Re-running with `Protocol::Connect` would yield
    // `application/proto`. Asserting both header and trailer keeps this
    // test honest if someone accidentally swaps the codec back to JSON.
    let response_ct = resp
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        response_ct.starts_with("application/grpc"),
        "expected gRPC content-type (binary proto framing); got {response_ct:?}"
    );
    let view = resp.into_view();
    assert_eq!(&*view.status, "ok");
    assert_eq!(&*view.protocol_version, "ohdc.v0");

    // ---- WhoAmI ----
    let resp = client
        .who_am_i(pb::WhoAmIRequest::default())
        .await
        .expect("whoami");
    let view = resp.into_view();
    assert_eq!(&*view.token_kind, "self_session");

    // ---- PutEvents ----
    let event = pb::EventInput {
        timestamp_ms: 1_700_000_000_000_i64,
        event_type: "std.blood_glucose".into(),
        channels: vec![pb::ChannelValue {
            channel_path: "value".into(),
            value: Some(pb::channel_value::Value::RealValue(6.7)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let resp = client
        .put_events(pb::PutEventsRequest {
            events: vec![event],
            atomic: false,
            ..Default::default()
        })
        .await
        .expect("put_events");
    let owned = resp.into_owned();
    assert_eq!(owned.results.len(), 1);
    let first = &owned.results[0];
    let outcome = first.outcome.as_ref().expect("outcome set");
    let committed = match outcome {
        pb::put_event_result::Outcome::Committed(b) => b.as_ref(),
        other => panic!("expected committed, got {other:?}"),
    };
    let ulid_pb = committed.ulid.as_option().expect("ulid set");
    assert_eq!(ulid_pb.bytes.len(), 16);
    let mut ulid_bytes = [0u8; 16];
    ulid_bytes.copy_from_slice(&ulid_pb.bytes);

    // ---- QueryEvents (server-streaming) ----
    let mut stream = client
        .query_events(pb::QueryEventsRequest {
            filter: ::buffa::MessageField::some(pb::EventFilter {
                event_types_in: vec!["std.blood_glucose".into()],
                include_superseded: true,
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect("query_events");
    let mut count = 0;
    while let Some(view) = stream.message().await.expect("query stream") {
        count += 1;
        let event_ulid = view.ulid.as_option().expect("ulid set");
        assert_eq!(event_ulid.bytes, ulid_bytes.to_vec());
        assert_eq!(&*view.event_type, "std.blood_glucose");
    }
    assert_eq!(count, 1, "expected exactly one matching event");

    // ---- GetEventByUlid ----
    let resp = client
        .get_event_by_ulid(pb::GetEventByUlidRequest {
            ulid: ::buffa::MessageField::some(pb::Ulid {
                bytes: ulid_bytes.to_vec(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .expect("get_event_by_ulid");
    let view = resp.into_view();
    assert_eq!(&*view.event_type, "std.blood_glucose");
    let evid = view.ulid.as_option().expect("ulid set");
    assert_eq!(evid.bytes, ulid_bytes.to_vec());

    // ---- Sanity on encoding the ULID round-trips Crockford. ----
    let crockford = ohd_ulid::to_crockford(&ulid_bytes);
    assert_eq!(ohd_ulid::parse_crockford(&crockford).unwrap(), ulid_bytes);

    // ---- Unauthenticated WhoAmI returns Unauthenticated. ----
    let no_auth_config = ClientConfig::new(uri.clone());
    let no_auth_conn = Http2Connection::connect_plaintext(no_auth_config.base_uri.clone())
        .await
        .expect("h2 connect (no-auth)")
        .shared(64);
    let anon = OhdcServiceClient::new(no_auth_conn, no_auth_config);
    let err: ConnectError = anon
        .who_am_i(pb::WhoAmIRequest::default())
        .await
        .err()
        .expect("expected Unauthenticated");
    assert_eq!(err.code, connectrpc::ErrorCode::Unauthenticated);

    // ----------------------------------------------------------------
    // Pending-event flow: drive PutEvents under a grant token whose
    // approval_mode='always', then ListPending + ApprovePending over the
    // wire. Confirms the full pending-flow trio is reachable as wire RPCs
    // (replacing the deprecated `pending-list` / `pending-approve` CLI
    // helpers in main.rs).
    // ----------------------------------------------------------------
    let grant_id = storage
        .with_conn_mut(|conn| {
            create_grant(
                conn,
                &NewGrant {
                    grantee_label: "Dr. E2E".into(),
                    grantee_kind: "human".into(),
                    purpose: Some("e2e test".into()),
                    default_action: RuleEffect::Deny,
                    approval_mode: "always".into(),
                    write_event_type_rules: vec![("std.blood_glucose".into(), RuleEffect::Allow)],
                    event_type_rules: vec![("std.blood_glucose".into(), RuleEffect::Allow)],
                    ..Default::default()
                },
            )
            .map(|(id, _u)| id)
        })
        .unwrap();
    let grant_bearer = storage
        .with_conn(|conn| issue_grant_token(conn, user_ulid, grant_id, TokenKind::Grant, None))
        .unwrap();

    let grant_conn = Http2Connection::connect_plaintext(uri.clone())
        .await
        .expect("h2 connect (grant)")
        .shared(64);
    let grant_config = ClientConfig::new(uri.clone())
        .protocol(connectrpc::Protocol::Grpc)
        .default_header("authorization", format!("Bearer {grant_bearer}"));
    let grant_client = OhdcServiceClient::new(grant_conn, grant_config);

    // Doctor submits a glucose event — should queue.
    let doctor_event = pb::EventInput {
        timestamp_ms: 1_700_000_005_000_i64,
        event_type: "std.blood_glucose".into(),
        channels: vec![pb::ChannelValue {
            channel_path: "value".into(),
            value: Some(pb::channel_value::Value::RealValue(8.4)),
            ..Default::default()
        }],
        ..Default::default()
    };
    let resp = grant_client
        .put_events(pb::PutEventsRequest {
            events: vec![doctor_event],
            atomic: false,
            ..Default::default()
        })
        .await
        .expect("grant put_events");
    let put = resp.into_owned();
    assert_eq!(put.results.len(), 1);
    let pending_ulid_pb = match put.results[0].outcome.as_ref().expect("outcome") {
        pb::put_event_result::Outcome::Pending(p) => p.ulid.as_option().expect("ulid").clone(),
        other => panic!("expected pending, got {other:?}"),
    };
    assert_eq!(pending_ulid_pb.bytes.len(), 16);

    // User (self-session) lists pending → sees the doctor's submission.
    let resp = client
        .list_pending(pb::ListPendingRequest {
            status: Some("pending".into()),
            ..Default::default()
        })
        .await
        .expect("list pending");
    let listed = resp.into_owned();
    assert_eq!(listed.pending.len(), 1);
    assert_eq!(listed.pending[0].status, "pending");
    let listed_ulid = listed.pending[0].ulid.as_option().expect("ulid");
    assert_eq!(listed_ulid.bytes, pending_ulid_pb.bytes);

    // User approves → ApprovePending returns the same ULID, with a positive
    // commit timestamp.
    let resp = client
        .approve_pending(pb::ApprovePendingRequest {
            pending_ulid: ::buffa::MessageField::some(pending_ulid_pb.clone()),
            also_auto_approve_this_type: false,
            ..Default::default()
        })
        .await
        .expect("approve pending");
    let approved = resp.into_owned();
    assert!(approved.committed_at_ms > 0);
    let approved_ulid = approved.event_ulid.as_option().expect("event ulid");
    assert_eq!(approved_ulid.bytes, pending_ulid_pb.bytes);

    // ListGrants over the wire (self-session sees both the demo grant
    // created via the storage core helper above plus any others; we just
    // verify ≥ 1).
    let resp = client
        .list_grants(pb::ListGrantsRequest::default())
        .await
        .expect("list grants");
    let listed = resp.into_owned();
    assert!(!listed.grants.is_empty(), "expected at least one grant row");

    // ----------------------------------------------------------------
    // Pending-query flow: require_approval_per_query should expose the
    // read approval queue over List/ApprovePendingQuery, then let the
    // exact same query succeed after approval.
    // ----------------------------------------------------------------
    let query_grant_id = storage
        .with_conn_mut(|conn| {
            create_grant(
                conn,
                &NewGrant {
                    grantee_label: "Query reviewer".into(),
                    grantee_kind: "human".into(),
                    purpose: Some("pending-query e2e".into()),
                    default_action: RuleEffect::Deny,
                    approval_mode: "never_required".into(),
                    require_approval_per_query: true,
                    event_type_rules: vec![("std.blood_glucose".into(), RuleEffect::Allow)],
                    ..Default::default()
                },
            )
            .map(|(id, _u)| id)
        })
        .unwrap();
    let query_grant_bearer = storage
        .with_conn(|conn| {
            issue_grant_token(conn, user_ulid, query_grant_id, TokenKind::Grant, None)
        })
        .unwrap();
    let query_grant_conn = Http2Connection::connect_plaintext(uri.clone())
        .await
        .expect("h2 connect (query grant)")
        .shared(64);
    let query_grant_config = ClientConfig::new(uri.clone())
        .protocol(connectrpc::Protocol::Grpc)
        .default_header("authorization", format!("Bearer {query_grant_bearer}"));
    let query_grant_client = OhdcServiceClient::new(query_grant_conn, query_grant_config);
    let pending_query_request = pb::QueryEventsRequest {
        filter: ::buffa::MessageField::some(pb::EventFilter {
            event_types_in: vec!["std.blood_glucose".into()],
            include_superseded: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let err: ConnectError = query_grant_client
        .query_events(pending_query_request.clone())
        .await
        .err()
        .expect("expected pending approval");
    assert_eq!(err.code, connectrpc::ErrorCode::FailedPrecondition);
    assert!(
        err.to_string().contains("PENDING_APPROVAL"),
        "expected PENDING_APPROVAL, got {err}"
    );

    let mut pending_queries = client
        .list_pending_queries(pb::ListPendingQueriesRequest {
            include_decided: false,
            ..Default::default()
        })
        .await
        .expect("list pending queries");
    let mut query_ulid_pb = None;
    let mut pending_query_count = 0;
    while let Some(view) = pending_queries
        .message()
        .await
        .expect("pending-query stream")
    {
        pending_query_count += 1;
        assert!(view.query_payload.len() > 0);
        let query_ulid = view.query_ulid.as_option().expect("query ulid");
        query_ulid_pb = Some(pb::Ulid {
            bytes: query_ulid.bytes.to_vec(),
            ..Default::default()
        });
    }
    assert_eq!(pending_query_count, 1);
    let query_ulid_pb = query_ulid_pb.expect("pending query ulid");

    let approved = client
        .approve_pending_query(pb::ApprovePendingQueryRequest {
            query_ulid: ::buffa::MessageField::some(query_ulid_pb),
            ..Default::default()
        })
        .await
        .expect("approve pending query")
        .into_owned();
    assert!(approved.ok);

    let mut stream = query_grant_client
        .query_events(pending_query_request)
        .await
        .expect("query succeeds after approval");
    let mut approved_count = 0;
    while let Some(_view) = stream.message().await.expect("approved query stream") {
        approved_count += 1;
    }
    assert!(approved_count >= 1);

    let reject_query_request = pb::QueryEventsRequest {
        filter: ::buffa::MessageField::some(pb::EventFilter {
            event_types_in: vec!["std.blood_glucose".into()],
            source_in: vec!["pending-query-reject-path".into()],
            include_superseded: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let err: ConnectError = query_grant_client
        .query_events(reject_query_request.clone())
        .await
        .err()
        .expect("expected pending approval before reject");
    assert_eq!(err.code, connectrpc::ErrorCode::FailedPrecondition);

    let mut pending_queries = client
        .list_pending_queries(pb::ListPendingQueriesRequest {
            include_decided: false,
            ..Default::default()
        })
        .await
        .expect("list pending query to reject");
    let reject_ulid_pb = pending_queries
        .message()
        .await
        .expect("pending-query stream")
        .expect("one pending query")
        .query_ulid
        .as_option()
        .expect("query ulid")
        .to_owned();
    let reject_ulid_pb = pb::Ulid {
        bytes: reject_ulid_pb.bytes.to_vec(),
        ..Default::default()
    };
    assert!(
        pending_queries
            .message()
            .await
            .expect("pending-query stream")
            .is_none(),
        "expected only the query being rejected to remain pending"
    );
    let rejected = client
        .reject_pending_query(pb::RejectPendingQueryRequest {
            query_ulid: ::buffa::MessageField::some(reject_ulid_pb),
            reason: Some("wire e2e reject".into()),
            ..Default::default()
        })
        .await
        .expect("reject pending query")
        .into_owned();
    assert!(rejected.ok);
    let err: ConnectError = query_grant_client
        .query_events(reject_query_request)
        .await
        .err()
        .expect("expected rejected query to stay denied");
    assert_eq!(err.code, connectrpc::ErrorCode::PermissionDenied);

    server_handle.abort();
}
