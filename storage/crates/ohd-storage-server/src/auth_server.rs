//! Connect-RPC handlers for the AuthService.
//!
//! Implements the multi-identity account-linking RPCs per
//! `spec/auth.md` "Multiple identities per user" + STATUS.md
//! "Multi-identity account linking":
//!
//! - `ListIdentities`         — return every OIDC identity bound to the caller.
//! - `LinkIdentityStart`      — mint a `link_token` (10-minute TTL).
//! - `CompleteIdentityLink`   — verify an id_token + insert the new identity.
//! - `UnlinkIdentity`         — refuse if it would orphan the user.
//! - `SetPrimaryIdentity`     — promote one identity, demote the previous primary.
//!
//! The remaining AuthService RPCs (sessions / invites / device tokens /
//! notifications / push registration) are intentionally returned as
//! `Unimplemented` from this adapter for v1 — they're scaffolded in
//! `proto/ohdc/v0/auth.proto` but the handler bodies are deferred until the
//! deployment-level system DB lands. See STATUS.md.
//!
//! Auth profile rules: only **self-session** tokens may manage identities.
//! Grant tokens have no business linking new identities to a user, and the
//! delegate-grant pattern is read/write of *data*, not identity metadata.

use std::sync::Arc;

use buffa::MessageField;
use connectrpc::{
    ConnectError, ErrorCode, RequestContext, Response as ConnectResponse, ServiceResult,
};
use ohd_storage_core::{
    audit::{self, ActorType, AuditEntry, AuditResult},
    auth::{self as ohd_auth, ResolvedToken, TokenKind},
    identities::{
        self as ohd_identities, Identity as CoreIdentity, IssuerVerification, JwksResolver,
    },
    storage::Storage,
    Error,
};

use crate::proto::ohdc::v0 as pb;
use crate::proto::ohdc::v0::AuthService;
use crate::proto::ohdc::v0::AuthServiceExt;

/// AuthService impl.
///
/// Holds the same `Arc<Storage>` as the OhdcService and reads/writes the
/// `_oidc_identities` + `_pending_identity_links` tables. The JWKS resolver
/// is injected so production wires a HTTP fetcher and tests inject in-memory
/// keys without going through the network.
#[derive(Clone)]
pub struct AuthAdapter {
    storage: Arc<Storage>,
    jwks: Arc<dyn JwksResolver>,
}

impl AuthAdapter {
    /// Construct an adapter with a custom JWKS resolver.
    pub fn new(storage: Arc<Storage>, jwks: Arc<dyn JwksResolver>) -> Self {
        Self { storage, jwks }
    }
}

/// Register the `AuthService` against an existing connectrpc Router.
pub fn register_auth(
    storage: Arc<Storage>,
    jwks: Arc<dyn JwksResolver>,
    router: connectrpc::Router,
) -> connectrpc::Router {
    let svc = Arc::new(AuthAdapter::new(storage, jwks));
    svc.register(router)
}

fn require_self_session(
    adapter: &AuthAdapter,
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
            "WRONG_TOKEN_KIND: AuthService identity-management RPCs require a self-session token",
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
        413 => ErrorCode::ResourceExhausted,
        429 => ErrorCode::ResourceExhausted,
        503 => ErrorCode::Unavailable,
        _ => ErrorCode::Internal,
    };
    let ohdc_code = err.code();
    ConnectError::new(code, format!("{ohdc_code}: {err}"))
}

fn identity_to_pb(id: &CoreIdentity, include_subject: bool) -> pb::Identity {
    pb::Identity {
        provider: id.provider.clone(),
        // The subject is omitted from the WhoAmI list (no PII leak) but
        // included in ListIdentities (the caller is the user themselves —
        // they can see their own subject claim).
        subject: if include_subject {
            id.subject.clone()
        } else {
            String::new()
        },
        email: None,
        linked_at_ms: id.linked_at_ms,
        display_label: id.display_label.clone(),
        is_primary: id.is_primary,
        last_login_ms: id.last_login_ms,
        ..Default::default()
    }
}

fn audit_action(
    storage: &Storage,
    token: &ResolvedToken,
    action: &str,
    result: AuditResult,
    reason: Option<String>,
) {
    let _ = storage.with_conn(|conn| {
        audit::append(
            conn,
            &AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: ActorType::Self_,
                auto_granted: false,
                grant_id: token.grant_id,
                action: action.into(),
                query_kind: None,
                query_params_json: None,
                rows_returned: None,
                rows_filtered: None,
                result,
                reason,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    });
}

impl AuthService for AuthAdapter {
    // ---- ListIdentities ---------------------------------------------------

    fn list_identities<'a>(
        &'a self,
        ctx: RequestContext,
        _request: pb::OwnedListIdentitiesRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::ListIdentitiesResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_self_session(self, &ctx)?;
            let user = token.effective_user_ulid();
            let rows = self
                .storage
                .with_conn(|conn| ohd_identities::list_identities(conn, user))
                .map_err(error_to_connect)?;
            audit_action(
                &self.storage,
                &token,
                "auth.list_identities",
                AuditResult::Success,
                None,
            );
            let identities: Vec<pb::Identity> =
                rows.iter().map(|i| identity_to_pb(i, true)).collect();
            Ok(ConnectResponse::new(pb::ListIdentitiesResponse {
                identities,
                ..Default::default()
            }))
        }
    }

    // ---- LinkIdentityStart ------------------------------------------------

    fn link_identity_start<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedLinkIdentityStartRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::LinkIdentityStartResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_self_session(self, &ctx)?;
            let req = request.to_owned_message();
            let provider_hint = if req.provider_hint.is_empty() {
                None
            } else {
                Some(req.provider_hint.as_str())
            };
            let outcome = self
                .storage
                .with_conn(|conn| {
                    ohd_identities::link_identity_start(conn, token.user_ulid, None, provider_hint)
                })
                .map_err(error_to_connect)?;
            audit_action(
                &self.storage,
                &token,
                "auth.link_identity_start",
                AuditResult::Success,
                provider_hint.map(str::to_string),
            );
            Ok(ConnectResponse::new(pb::LinkIdentityStartResponse {
                link_token: outcome.link_token,
                oauth_url: String::new(),
                expires_at_ms: outcome.expires_at_ms,
                ..Default::default()
            }))
        }
    }

    // ---- CompleteIdentityLink --------------------------------------------

    fn complete_identity_link<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedCompleteIdentityLinkRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::CompleteIdentityLinkResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_self_session(self, &ctx)?;
            let req = request.to_owned_message();
            if req.link_token.is_empty() {
                return Err(ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: link_token required",
                ));
            }
            if req.id_token.is_empty() {
                return Err(ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: id_token required",
                ));
            }
            if req.issuer.is_empty() {
                return Err(ConnectError::new(
                    ErrorCode::InvalidArgument,
                    "INVALID_ARGUMENT: issuer required",
                ));
            }
            let cfg = IssuerVerification::new(req.issuer.clone(), req.audiences.clone());
            let display_label_opt = req.display_label.clone();
            let display_label = display_label_opt.as_deref();
            let jwks = Arc::clone(&self.jwks);
            let result = self.storage.with_conn_mut(|conn| {
                ohd_identities::complete_identity_link(
                    conn,
                    &req.link_token,
                    &req.id_token,
                    &cfg,
                    jwks.as_ref(),
                    display_label,
                )
            });
            match result {
                Ok(identity) => {
                    audit_action(
                        &self.storage,
                        &token,
                        "auth.complete_identity_link",
                        AuditResult::Success,
                        Some(identity.provider.clone()),
                    );
                    Ok(ConnectResponse::new(pb::CompleteIdentityLinkResponse {
                        identity: MessageField::some(identity_to_pb(&identity, true)),
                        ..Default::default()
                    }))
                }
                Err(e) => {
                    audit_action(
                        &self.storage,
                        &token,
                        "auth.complete_identity_link",
                        AuditResult::Rejected,
                        Some(e.to_string()),
                    );
                    Err(error_to_connect(e))
                }
            }
        }
    }

    // ---- UnlinkIdentity ---------------------------------------------------

    fn unlink_identity<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedUnlinkIdentityRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::UnlinkIdentityResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_self_session(self, &ctx)?;
            let req = request.to_owned_message();
            let user = token.effective_user_ulid();
            let now = audit::now_ms();
            let res = self.storage.with_conn_mut(|conn| {
                ohd_identities::unlink_identity(conn, user, &req.provider, &req.subject)
            });
            match res {
                Ok(()) => {
                    audit_action(
                        &self.storage,
                        &token,
                        "auth.unlink_identity",
                        AuditResult::Success,
                        Some(format!("{}/{}", req.provider, req.subject)),
                    );
                    Ok(ConnectResponse::new(pb::UnlinkIdentityResponse {
                        unlinked_at_ms: now,
                        ..Default::default()
                    }))
                }
                Err(Error::OutOfScope) => {
                    audit_action(
                        &self.storage,
                        &token,
                        "auth.unlink_identity",
                        AuditResult::Rejected,
                        Some("LAST_IDENTITY_PROTECTED".into()),
                    );
                    Err(ConnectError::new(
                        ErrorCode::PermissionDenied,
                        "LAST_IDENTITY_PROTECTED: cannot unlink the last identity bound to this user",
                    ))
                }
                Err(e) => Err(error_to_connect(e)),
            }
        }
    }

    // ---- SetPrimaryIdentity -----------------------------------------------

    fn set_primary_identity<'a>(
        &'a self,
        ctx: RequestContext,
        request: pb::OwnedSetPrimaryIdentityRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::SetPrimaryIdentityResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            let token = require_self_session(self, &ctx)?;
            let req = request.to_owned_message();
            let user = token.effective_user_ulid();
            let now = audit::now_ms();
            self.storage
                .with_conn_mut(|conn| {
                    ohd_identities::set_primary(conn, user, &req.provider, &req.subject)
                })
                .map_err(error_to_connect)?;
            audit_action(
                &self.storage,
                &token,
                "auth.set_primary_identity",
                AuditResult::Success,
                Some(format!("{}/{}", req.provider, req.subject)),
            );
            Ok(ConnectResponse::new(pb::SetPrimaryIdentityResponse {
                updated_at_ms: now,
                ..Default::default()
            }))
        }
    }

    // ============================================================================
    // The rest of the AuthService surface is deferred to v1.x. The proto's
    // generated trait requires every method, so we stub them as
    // `Unimplemented`. See STATUS.md.
    // ============================================================================

    fn list_sessions<'a>(
        &'a self,
        _ctx: RequestContext,
        _request: pb::OwnedListSessionsRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::ListSessionsResponse> + Send + use<'a>,
        >,
    > + Send {
        async move { unimpl::<pb::ListSessionsResponse>("ListSessions") }
    }

    fn revoke_session<'a>(
        &'a self,
        _ctx: RequestContext,
        _request: pb::OwnedRevokeSessionRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::RevokeSessionResponse> + Send + use<'a>,
        >,
    > + Send {
        async move { unimpl::<pb::RevokeSessionResponse>("RevokeSession") }
    }

    fn logout<'a>(
        &'a self,
        _ctx: RequestContext,
        _request: pb::OwnedLogoutRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<impl connectrpc::Encodable<pb::LogoutResponse> + Send + use<'a>>,
    > + Send {
        async move { unimpl::<pb::LogoutResponse>("Logout") }
    }

    fn logout_everywhere<'a>(
        &'a self,
        _ctx: RequestContext,
        _request: pb::OwnedLogoutEverywhereRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::LogoutEverywhereResponse> + Send + use<'a>,
        >,
    > + Send {
        async move { unimpl::<pb::LogoutEverywhereResponse>("LogoutEverywhere") }
    }

    fn issue_invite<'a>(
        &'a self,
        _ctx: RequestContext,
        _request: pb::OwnedIssueInviteRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::IssueInviteResponse> + Send + use<'a>,
        >,
    > + Send {
        async move { unimpl::<pb::IssueInviteResponse>("IssueInvite") }
    }

    fn list_invites<'a>(
        &'a self,
        _ctx: RequestContext,
        _request: pb::OwnedListInvitesRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::ListInvitesResponse> + Send + use<'a>,
        >,
    > + Send {
        async move { unimpl::<pb::ListInvitesResponse>("ListInvites") }
    }

    fn revoke_invite<'a>(
        &'a self,
        _ctx: RequestContext,
        _request: pb::OwnedRevokeInviteRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::RevokeInviteResponse> + Send + use<'a>,
        >,
    > + Send {
        async move { unimpl::<pb::RevokeInviteResponse>("RevokeInvite") }
    }

    fn issue_device_token<'a>(
        &'a self,
        _ctx: RequestContext,
        _request: pb::OwnedIssueDeviceTokenRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::IssueDeviceTokenResponse> + Send + use<'a>,
        >,
    > + Send {
        async move { unimpl::<pb::IssueDeviceTokenResponse>("IssueDeviceToken") }
    }

    fn register_push_token<'a>(
        &'a self,
        _ctx: RequestContext,
        _request: pb::OwnedRegisterPushTokenRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::RegisterPushTokenResponse> + Send + use<'a>,
        >,
    > + Send {
        async move { unimpl::<pb::RegisterPushTokenResponse>("RegisterPushToken") }
    }

    fn update_notification_preferences<'a>(
        &'a self,
        _ctx: RequestContext,
        _request: pb::OwnedUpdateNotificationPreferencesRequestView,
    ) -> impl std::future::Future<
        Output = ServiceResult<
            impl connectrpc::Encodable<pb::UpdateNotificationPreferencesResponse> + Send + use<'a>,
        >,
    > + Send {
        async move {
            unimpl::<pb::UpdateNotificationPreferencesResponse>("UpdateNotificationPreferences")
        }
    }
}

/// Return an `Err` ServiceResult typed for an `Ok` arm of `T` so the
/// async-block `impl Encodable<T>` bound resolves to `T` itself (proto
/// messages auto-impl `Encodable<Self>`). The Err is the actual result the
/// caller sees.
fn unimpl<T>(rpc: &'static str) -> ServiceResult<T> {
    Err(ConnectError::new(
        ErrorCode::Unimplemented,
        format!("UNIMPLEMENTED: {rpc} is deferred to v1.x"),
    ))
}

/// Public alias — exposes `Identity` proto conversion to the OhdcService
/// adapter so `WhoAmI` can include the caller's linked identities.
#[allow(dead_code)]
pub fn identity_to_pb_pub(id: &CoreIdentity, include_subject: bool) -> pb::Identity {
    identity_to_pb(id, include_subject)
}
