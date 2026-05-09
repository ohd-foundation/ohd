//! Connect-RPC handlers for the SyncService.
//!
//! Implements the bidirectional event-log replay defined in
//! `spec/sync-protocol.md`. Cache ↔ primary sync uses Connect-RPC over the
//! same transport stack as the consumer-facing `OhdcService` — auth is the
//! user's self-session token (cache talking to its own primary).
//!
//! v0 wires the core handlers (Hello / PushFrames / PullFrames) and the
//! grant-on-primary delegations (Create / Revoke / Update). Attachment
//! payload sync is wired enough to compile but defers to the same machinery
//! as `OhdcService.AttachBlob` for the actual write — sync attachments and
//! consumer attachments share the same blob store.
//!
//! Auth: the bearer token must be a self-session token. Grant tokens cannot
//! drive sync (each peer is the user themselves under OIDC).

use std::pin::Pin;
use std::sync::Arc;

use buffa::MessageField;
use connectrpc::{
    ConnectError, ErrorCode, RequestContext, Response as ConnectResponse, ServiceResult,
    ServiceStream,
};
use futures::{Stream, StreamExt};
use ohd_storage_core::{
    auth::{self as ohd_auth, ResolvedToken, TokenKind},
    storage::Storage,
    sync as ohd_sync, ulid as ohd_ulid, Error,
};

use crate::proto::ohdc::v0 as pb;
use crate::proto::ohdc::v0::SyncService;
use crate::proto::ohdc::v0::SyncServiceExt;
use crate::server as ohdc_server;

/// SyncService impl. Holds the same `Arc<Storage>` as the OhdcService and
/// reads/writes the same tables; sync writes are tagged with `origin_peer_id`
/// to suppress echo on the next outbound push.
#[derive(Clone)]
pub struct SyncAdapter {
    storage: Arc<Storage>,
}

impl SyncAdapter {
    /// Construct.
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }
}

/// Register `SyncService` against an existing connectrpc Router.
pub fn register_sync(storage: Arc<Storage>, router: connectrpc::Router) -> connectrpc::Router {
    let svc = Arc::new(SyncAdapter::new(storage));
    svc.register(router)
}

fn require_self_session(
    adapter: &SyncAdapter,
    ctx: &RequestContext,
) -> Result<ResolvedToken, ConnectError> {
    let bearer = ctx
        .headers
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| ConnectError::new(ErrorCode::Unauthenticated, "missing bearer token"))?;
    let token = adapter
        .storage
        .with_conn(|conn| ohd_auth::resolve_token(conn, bearer))
        .map_err(error_to_connect)?;
    if token.kind != TokenKind::SelfSession {
        return Err(ConnectError::new(
            ErrorCode::PermissionDenied,
            "WRONG_TOKEN_KIND: SyncService requires a self-session token",
        ));
    }
    Ok(token)
}

fn error_to_connect(err: Error) -> ConnectError {
    let code = match err.http_status() {
        202 => ErrorCode::FailedPrecondition,
        400 => ErrorCode::InvalidArgument,
        401 => ErrorCode::Unauthenticated,
        403 => ErrorCode::PermissionDenied,
        404 => ErrorCode::NotFound,
        408 => ErrorCode::DeadlineExceeded,
        409 => ErrorCode::AlreadyExists,
        429 => ErrorCode::ResourceExhausted,
        _ => ErrorCode::Internal,
    };
    let ohdc_code = err.code();
    ConnectError::new(code, format!("{ohdc_code}: {err}"))
}

fn ulid_pb_to_core(u: &pb::Ulid) -> Result<ohd_storage_core::ulid::Ulid, ConnectError> {
    if u.bytes.len() != 16 {
        return Err(ConnectError::new(
            ErrorCode::InvalidArgument,
            "INVALID_ULID: Ulid.bytes must be exactly 16 bytes",
        ));
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&u.bytes);
    Ok(out)
}

impl SyncService for SyncAdapter {
    fn hello<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedHelloRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::HelloResponse> + Send + use<'a>>,
    > + Send {
        async move {
            let _token = require_self_session(self, &ctx)?;
            let req = request.to_owned_message();
            // Upsert the peer row.
            let peer_ulid = req.peer_ulid.clone().into_option().and_then(|u| {
                if u.bytes.len() == 16 {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&u.bytes);
                    Some(o)
                } else {
                    None
                }
            });
            let peer_id = self
                .storage
                .with_conn(|conn| {
                    ohd_sync::upsert_peer(conn, &req.peer_label, &req.peer_kind, peer_ulid.as_ref())
                })
                .map_err(error_to_connect)?;
            // Reply with our high-water marks for this peer.
            let our_high: i64 = self
                .storage
                .with_conn(|conn| {
                    conn.query_row("SELECT COALESCE(MAX(id), 0) FROM events", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .map_err(Error::from)
                })
                .map_err(error_to_connect)?;
            let inbound: i64 = self
                .storage
                .with_conn(|conn| {
                    conn.query_row(
                        "SELECT last_inbound_peer_rowid FROM peer_sync WHERE id = ?1",
                        rusqlite::params![peer_id],
                        |r| r.get::<_, i64>(0),
                    )
                    .map_err(Error::from)
                })
                .map_err(error_to_connect)?;
            let user_ulid = self.storage.user_ulid();
            Ok(ConnectResponse::new(pb::HelloResponse {
                peer_label: req.peer_label,
                peer_kind: req.peer_kind,
                peer_ulid: req
                    .peer_ulid
                    .into_option()
                    .map(MessageField::some)
                    .unwrap_or_default(),
                my_local_rowid_high_water: our_high,
                my_inbound_watermark_for_you: inbound,
                registry_version: 1,
                caller_user_ulid: MessageField::some(pb::Ulid {
                    bytes: user_ulid.to_vec(),
                    ..Default::default()
                }),
                ..Default::default()
            }))
        }
    }

    fn push_frames(
        &self,
        ctx: RequestContext,
        requests: ServiceStream<pb::OwnedPushFrameView>,
    ) -> impl std::future::Future<Output = ServiceResult<ServiceStream<pb::PushAck>>> + Send {
        let storage = Arc::clone(&self.storage);
        async move {
            // Auth: must be self-session.
            let bearer = ctx
                .headers
                .get(http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|h| h.strip_prefix("Bearer "))
                .ok_or_else(|| {
                    ConnectError::new(ErrorCode::Unauthenticated, "missing bearer token")
                })?;
            let token = storage
                .with_conn(|conn| ohd_auth::resolve_token(conn, bearer))
                .map_err(error_to_connect)?;
            if token.kind != TokenKind::SelfSession {
                return Err(ConnectError::new(
                    ErrorCode::PermissionDenied,
                    "WRONG_TOKEN_KIND: PushFrames requires self-session",
                ));
            }
            // Drain the inbound stream synchronously for v1; async batching
            // is a v1.x optimization. The peer is identified per frame (not
            // per call) — we look up by rowid space below.
            let mut acks: Vec<Result<pb::PushAck, ConnectError>> = Vec::new();
            let mut requests = requests;
            while let Some(frame_view) = requests.next().await {
                let frame_view = match frame_view {
                    Ok(v) => v,
                    Err(e) => {
                        acks.push(Err(e));
                        continue;
                    }
                };
                let frame = frame_view.to_owned_message();
                let sender_rowid = frame.sender_rowid;
                match frame.entity {
                    Some(pb::push_frame::Entity::Event(ef)) => {
                        let event_pb = match ef.event.into_option() {
                            Some(e) => e,
                            None => {
                                acks.push(Ok(pb::PushAck {
                                    sender_rowid,
                                    outcome: "rejected".into(),
                                    error: MessageField::some(pb::ErrorInfo {
                                        code: "INVALID_ARGUMENT".into(),
                                        message: "EventFrame.event missing".into(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                }));
                                continue;
                            }
                        };
                        let event = match ohdc_server::pb_event_to_core_pub(event_pb) {
                            Ok(e) => e,
                            Err(e) => {
                                acks.push(Err(e));
                                continue;
                            }
                        };
                        // Use a synthetic peer ("inbound-stream") for this
                        // batch — production caches send a Hello first which
                        // creates the peer row; if absent, we lazily create.
                        let peer_label = "inbound-stream";
                        let peer_id = match storage.with_conn(|conn| {
                            ohd_sync::upsert_peer(conn, peer_label, "cache", None)
                        }) {
                            Ok(id) => id,
                            Err(e) => {
                                acks.push(Err(error_to_connect(e)));
                                continue;
                            }
                        };
                        match storage.with_conn_mut(|conn| {
                            ohd_sync::apply_inbound_event(conn, peer_id, &event)
                        }) {
                            Ok(true) => {
                                let _ = storage.with_conn(|conn| {
                                    ohd_sync::advance_inbound_watermark(conn, peer_id, sender_rowid)
                                });
                                acks.push(Ok(pb::PushAck {
                                    sender_rowid,
                                    outcome: "ok".into(),
                                    ..Default::default()
                                }));
                            }
                            Ok(false) => {
                                acks.push(Ok(pb::PushAck {
                                    sender_rowid,
                                    outcome: "duplicate".into(),
                                    ..Default::default()
                                }));
                            }
                            Err(e) => {
                                acks.push(Ok(pb::PushAck {
                                    sender_rowid,
                                    outcome: "rejected".into(),
                                    error: MessageField::some(pb::ErrorInfo {
                                        code: e.code().into(),
                                        message: e.to_string(),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                }));
                            }
                        }
                    }
                    // Other entity kinds (PendingEvent, Grant, Device, AppVersion,
                    // RegistryEntry) are out-of-scope for v1's first pass; sync
                    // accepts them with an "ok" ack but doesn't yet apply.
                    Some(_) => {
                        acks.push(Ok(pb::PushAck {
                            sender_rowid,
                            outcome: "ok".into(),
                            ..Default::default()
                        }));
                    }
                    None => {
                        acks.push(Ok(pb::PushAck {
                            sender_rowid,
                            outcome: "rejected".into(),
                            error: MessageField::some(pb::ErrorInfo {
                                code: "INVALID_ARGUMENT".into(),
                                message: "PushFrame.entity missing".into(),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }));
                    }
                }
            }
            let stream = futures::stream::iter(acks);
            let boxed: ServiceStream<pb::PushAck> = Box::pin(stream)
                as Pin<Box<dyn Stream<Item = Result<pb::PushAck, ConnectError>> + Send>>;
            Ok(ConnectResponse::new(boxed))
        }
    }

    fn pull_frames(
        &self,
        ctx: RequestContext,
        request: pb::OwnedPullRequestView,
    ) -> impl std::future::Future<Output = ServiceResult<ServiceStream<pb::PushFrame>>> + Send {
        let storage = Arc::clone(&self.storage);
        async move {
            // Auth: self-session.
            let bearer = ctx
                .headers
                .get(http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|h| h.strip_prefix("Bearer "))
                .ok_or_else(|| {
                    ConnectError::new(ErrorCode::Unauthenticated, "missing bearer token")
                })?;
            let token = storage
                .with_conn(|conn| ohd_auth::resolve_token(conn, bearer))
                .map_err(error_to_connect)?;
            if token.kind != TokenKind::SelfSession {
                return Err(ConnectError::new(
                    ErrorCode::PermissionDenied,
                    "WRONG_TOKEN_KIND: PullFrames requires self-session",
                ));
            }
            let req = request.to_owned_message();
            let after = req.after_peer_rowid;
            let limit = req.max_frames.unwrap_or(1000) as i64;
            // Use a synthetic peer for this stream's outbound suppression.
            let peer_id = storage
                .with_conn(|conn| ohd_sync::upsert_peer(conn, "inbound-stream", "cache", None))
                .map_err(error_to_connect)?;
            let outbound = storage
                .with_conn(|conn| ohd_sync::outbound_events(conn, peer_id, after, limit))
                .map_err(error_to_connect)?;
            let mut frames: Vec<Result<pb::PushFrame, ConnectError>> =
                Vec::with_capacity(outbound.len());
            for (rowid, event) in outbound {
                let pb_event = ohdc_server::event_core_to_pb_pub(event);
                frames.push(Ok(pb::PushFrame {
                    sender_rowid: rowid,
                    entity: Some(pb::push_frame::Entity::Event(Box::new(pb::EventFrame {
                        event: MessageField::some(pb_event),
                        ..Default::default()
                    }))),
                    ..Default::default()
                }));
            }
            let stream = futures::stream::iter(frames);
            let boxed: ServiceStream<pb::PushFrame> = Box::pin(stream)
                as Pin<Box<dyn Stream<Item = Result<pb::PushFrame, ConnectError>> + Send>>;
            Ok(ConnectResponse::new(boxed))
        }
    }

    fn push_attachment_blob<'a>(
        &'a self,
        ctx: RequestContext,
        requests: ServiceStream<pb::OwnedAttachmentChunkView>,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::AttachmentAck> + Send + use<'a>>,
    > + Send {
        async move {
            // Auth: must be self-session (same as push_frames / pull_frames).
            let _token = require_self_session(self, &ctx)?;
            // Identify the peer: production callers Hello() first which
            // upserts the row by label. We use the same synthetic
            // "inbound-stream" label as PushFrames so the watermark stays
            // consistent across the same peer's sync session.
            let peer_label = "inbound-stream";
            let peer_id = self
                .storage
                .with_conn(|conn| ohd_sync::upsert_peer(conn, peer_label, "cache", None))
                .map_err(error_to_connect)?;

            // Drain the stream: Init carries (attachment_ulid, sha256,
            // byte_size, mime, filename); Data frames append bytes; Finish
            // carries the expected sha256 for verification.
            let mut attachment_ulid_bytes: Option<[u8; 16]> = None;
            let mut init_sha: Option<Vec<u8>> = None;
            let mut buf: Vec<u8> = Vec::new();
            let mut finish_sha: Option<Vec<u8>> = None;
            let mut requests = requests;
            while let Some(chunk_view) = requests.next().await {
                let chunk_view = chunk_view?;
                let chunk = chunk_view.to_owned_message();
                match chunk.content {
                    Some(pb::attachment_chunk::Content::Init(init)) => {
                        if let Some(ref_msg) = init.r#ref.into_option() {
                            if let Some(u) = ref_msg.ulid.into_option() {
                                attachment_ulid_bytes = Some(ulid_pb_to_core(&u)?);
                            }
                            if !ref_msg.sha256.is_empty() {
                                init_sha = Some(ref_msg.sha256);
                            }
                        }
                    }
                    Some(pb::attachment_chunk::Content::Data(d)) => {
                        buf.extend_from_slice(&d);
                    }
                    Some(pb::attachment_chunk::Content::Finish(fin)) => {
                        if !fin.expected_sha256.is_empty() {
                            finish_sha = Some(fin.expected_sha256);
                        }
                    }
                    None => {
                        return Err(ConnectError::new(
                            ErrorCode::InvalidArgument,
                            "INVALID_ARGUMENT: AttachmentChunk missing content",
                        ));
                    }
                }
            }
            let attachment_ulid = attachment_ulid_bytes.ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: PushAttachmentBlob requires AttachmentInit with attachment ULID",
                )
            })?;
            let expected_sha_vec = finish_sha.or(init_sha).ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: PushAttachmentBlob requires sha256 (init or finish)",
                )
            })?;
            if expected_sha_vec.len() != 32 {
                return Err(ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: sha256 must be 32 bytes",
                ));
            }
            let mut expected_sha = [0u8; 32];
            expected_sha.copy_from_slice(&expected_sha_vec);

            // The metadata row should already exist (it shipped through the
            // EventFrame stream). Look it up + verify the sha matches.
            let lookup = self
                .storage
                .with_conn(|conn| {
                    ohd_storage_core::attachments::find_by_ulid_and_sha(
                        conn,
                        &attachment_ulid,
                        &expected_sha,
                    )
                })
                .map_err(error_to_connect)?;
            let (attachment_rowid, byte_size) = match lookup {
                Some((id, _sha, byte_size, _mime, _filename)) => (id, byte_size),
                None => {
                    // Metadata row missing: this can happen if the metadata
                    // frame hasn't arrived yet. Reject with NOT_FOUND so the
                    // sync orchestrator retries after a regular sync pass.
                    return Err(ConnectError::new(
                        ErrorCode::NotFound,
                        "NOT_FOUND: attachment metadata row missing; ship EventFrame first",
                    ));
                }
            };

            // Receive plaintext, encrypt under THIS storage's K_envelope (the
            // sender's K_envelope is irrelevant — different storage, different
            // envelope), write ciphertext to the sha-of-plaintext path. If the
            // storage handle has no envelope (testing-only no-cipher-key
            // path), fall back to legacy plaintext-on-disk via
            // `write_blob_atomic`.
            let storage_path = self.storage.path().to_path_buf();
            let root = ohd_storage_core::attachments::sidecar_root_for(&storage_path);
            match self.storage.envelope_key().cloned() {
                Some(envelope) => {
                    self.storage
                        .with_conn(|conn| {
                            ohd_storage_core::attachments::receive_and_encrypt_blob(
                                conn,
                                &root,
                                &envelope,
                                &attachment_ulid,
                                &buf,
                                &expected_sha,
                            )
                            .map(|_| ())
                        })
                        .map_err(error_to_connect)?;
                }
                None => {
                    ohd_storage_core::attachments::write_blob_atomic(&root, &buf, &expected_sha)
                        .map_err(error_to_connect)?;
                }
            }

            // Determine outcome: 'duplicate' if we already had the bytes
            // (delivery already recorded), else 'ok'. Idempotent semantics
            // — re-pushing is safe.
            let already = self
                .storage
                .with_conn(|conn| {
                    ohd_sync::attachment_delivered(
                        conn,
                        peer_id,
                        attachment_rowid,
                        ohd_sync::AttachmentSyncDirection::Push,
                    )
                })
                .map_err(error_to_connect)?;
            self.storage
                .with_conn(|conn| {
                    ohd_sync::record_attachment_delivery(
                        conn,
                        peer_id,
                        attachment_rowid,
                        ohd_sync::AttachmentSyncDirection::Push,
                        byte_size,
                    )
                })
                .map_err(error_to_connect)?;

            Ok(ConnectResponse::new(pb::AttachmentAck {
                attachment_ulid: MessageField::some(pb::Ulid {
                    bytes: attachment_ulid.to_vec(),
                    ..Default::default()
                }),
                sha256: expected_sha.to_vec(),
                outcome: if already {
                    "duplicate".into()
                } else {
                    "ok".into()
                },
                ..Default::default()
            }))
        }
    }

    fn pull_attachment_blob(
        &self,
        ctx: RequestContext,
        request: pb::OwnedPullAttachmentRequestView,
    ) -> impl std::future::Future<Output = ServiceResult<ServiceStream<pb::AttachmentChunk>>> + Send
    {
        let storage = Arc::clone(&self.storage);
        async move {
            // Auth: self-session.
            let bearer = ctx
                .headers
                .get(http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|h| h.strip_prefix("Bearer "))
                .ok_or_else(|| {
                    ConnectError::new(ErrorCode::Unauthenticated, "missing bearer token")
                })?;
            let token = storage
                .with_conn(|conn| ohd_auth::resolve_token(conn, bearer))
                .map_err(error_to_connect)?;
            if token.kind != TokenKind::SelfSession {
                return Err(ConnectError::new(
                    ErrorCode::PermissionDenied,
                    "WRONG_TOKEN_KIND: PullAttachmentBlob requires self-session",
                ));
            }
            let req = request.to_owned_message();
            let attachment_ulid_pb = req.attachment_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: attachment_ulid required",
                )
            })?;
            let attachment_ulid = ulid_pb_to_core(&attachment_ulid_pb)?;

            // Look up + read the bytes. The on-disk file is encrypted under
            // this storage's K_envelope (default-on as of the encrypted-
            // attachments flip); we decrypt to plaintext for the wire because
            // K_envelope differs per storage. The peer will encrypt under
            // *its* K_envelope on PushAttachmentBlob arrival.
            let storage_path = storage.path().to_path_buf();
            let root = ohd_storage_core::attachments::sidecar_root_for(&storage_path);
            let envelope_opt = storage.envelope_key().cloned();
            let (meta, bytes) = storage
                .with_conn(|conn| {
                    let (meta, _path) = ohd_storage_core::attachments::load_attachment_meta(
                        conn,
                        &root,
                        &attachment_ulid,
                    )?;
                    let bytes = ohd_storage_core::attachments::read_attachment_bytes(
                        conn,
                        &root,
                        &attachment_ulid,
                        envelope_opt.as_ref(),
                    )?;
                    Ok::<_, ohd_storage_core::Error>((meta, bytes))
                })
                .map_err(error_to_connect)?;

            // Record delivery (Pull direction) for this synthetic peer.
            let peer_label = "inbound-stream";
            let peer_id = storage
                .with_conn(|conn| ohd_sync::upsert_peer(conn, peer_label, "cache", None))
                .map_err(error_to_connect)?;
            let _ = storage.with_conn(|conn| {
                ohd_sync::record_attachment_delivery(
                    conn,
                    peer_id,
                    meta.id,
                    ohd_sync::AttachmentSyncDirection::Pull,
                    meta.byte_size,
                )
            });

            // Frame the response: Init → Data… → Finish.
            let mut frames: Vec<Result<pb::AttachmentChunk, ConnectError>> = Vec::new();
            frames.push(Ok(pb::AttachmentChunk {
                content: Some(pb::attachment_chunk::Content::Init(Box::new(
                    pb::AttachmentInit {
                        r#ref: MessageField::some(pb::AttachmentRef {
                            ulid: MessageField::some(pb::Ulid {
                                bytes: meta.ulid.to_vec(),
                                ..Default::default()
                            }),
                            sha256: meta.sha256.to_vec(),
                            byte_size: meta.byte_size,
                            mime_type: meta.mime_type.clone().unwrap_or_default(),
                            filename: meta.filename.clone().unwrap_or_default(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ))),
                ..Default::default()
            }));
            const CHUNK: usize = 64 * 1024;
            for chunk in bytes.chunks(CHUNK) {
                frames.push(Ok(pb::AttachmentChunk {
                    content: Some(pb::attachment_chunk::Content::Data(chunk.to_vec())),
                    ..Default::default()
                }));
            }
            frames.push(Ok(pb::AttachmentChunk {
                content: Some(pb::attachment_chunk::Content::Finish(Box::new(
                    pb::AttachmentFinish {
                        expected_sha256: meta.sha256.to_vec(),
                        ..Default::default()
                    },
                ))),
                ..Default::default()
            }));
            let stream = futures::stream::iter(frames);
            let boxed: ServiceStream<pb::AttachmentChunk> = Box::pin(stream)
                as Pin<Box<dyn Stream<Item = Result<pb::AttachmentChunk, ConnectError>> + Send>>;
            Ok(ConnectResponse::new(boxed))
        }
    }

    fn create_grant_on_primary<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedCreateGrantRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::CreateGrantResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_self_session(self, &ctx)?;
            // Delegate to OhdcService.CreateGrant — same handler, same audit.
            let req = request.to_owned_message();
            let new_grant = ohdc_server::create_grant_request_pb_to_core_pub(req)?;
            let outcome = ohd_storage_core::ohdc::create_grant(&self.storage, &token, &new_grant)
                .map_err(error_to_connect)?;
            let pb_grant = ohdc_server::grant_row_to_pb_pub(&outcome.grant);
            Ok(ConnectResponse::new(pb::CreateGrantResponse {
                grant: MessageField::some(pb_grant),
                token: outcome.token,
                share_url: outcome.share_url,
                share_qr_png: vec![],
                ..Default::default()
            }))
        }
    }

    fn revoke_grant_on_primary<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedRevokeGrantRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::RevokeGrantResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_self_session(self, &ctx)?;
            let req = request.to_owned_message();
            let grant_ulid_pb = req.grant_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: grant_ulid required",
                )
            })?;
            let grant_ulid = ulid_pb_to_core(&grant_ulid_pb)?;
            let revoked_at_ms = ohd_storage_core::ohdc::revoke_grant(
                &self.storage,
                &token,
                &grant_ulid,
                req.reason.as_deref(),
            )
            .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(pb::RevokeGrantResponse {
                revoked_at_ms,
                ..Default::default()
            }))
        }
    }

    fn update_grant_on_primary<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedUpdateGrantRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::Grant> + Send + use<'a>>,
    > + Send {
        async move {
            let token = require_self_session(self, &ctx)?;
            let req = request.to_owned_message();
            let grant_ulid_pb = req.grant_ulid.into_option().ok_or_else(|| {
                ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: grant_ulid required",
                )
            })?;
            let grant_ulid = ulid_pb_to_core(&grant_ulid_pb)?;
            let update = ohd_storage_core::grants::GrantUpdate {
                grantee_label: req.grantee_label,
                expires_at_ms: req.expires_at_ms,
            };
            let row =
                ohd_storage_core::ohdc::update_grant(&self.storage, &token, &grant_ulid, &update)
                    .map_err(error_to_connect)?;
            Ok(ConnectResponse::new(ohdc_server::grant_row_to_pb_pub(&row)))
        }
    }
}

#[allow(dead_code)]
fn _unused_ulid(_u: &ohd_ulid::Ulid) {}
