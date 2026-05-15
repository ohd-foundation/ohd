//! All HTTP endpoints. Each one is < 30 lines — the db module does the work.

use crate::auth::{mint_token, AuthedProfile};
use crate::db::OidcLink;
use crate::errors::{ApiError, ApiResult};
use crate::plans::{Plan, PlanInfo};
use crate::server::AppState;
use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

fn now_iso() -> String {
    OffsetDateTime::now_utc().format(&Rfc3339).unwrap_or_default()
}

// ===== POST /v1/account =====

#[derive(Deserialize)]
pub struct RegisterBody {
    pub profile_ulid: String,
    pub recovery_code: String,
}

#[derive(Serialize)]
pub struct RegisterResponse {
    pub profile_ulid: String,
    pub plan: Plan,
    pub access_token: String,
    pub created_at: String,
}

pub async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterBody>,
) -> ApiResult<Json<RegisterResponse>> {
    if body.profile_ulid.is_empty() || body.recovery_code.is_empty() {
        return Err(ApiError::BadRequest("profile_ulid and recovery_code required".into()));
    }
    let profile = state
        .db
        .register_profile(&body.profile_ulid, &body.recovery_code, &now_iso())?;
    let token = mint_token(&profile.profile_ulid, &state.config.jwt_secret, state.config.token_ttl_days)?;
    Ok(Json(RegisterResponse {
        profile_ulid: profile.profile_ulid,
        plan: profile.plan,
        access_token: token,
        created_at: profile.created_at,
    }))
}

// ===== GET /v1/account/me =====

#[derive(Serialize)]
pub struct MeResponse {
    pub profile_ulid: String,
    pub plan: Plan,
    pub created_at: String,
    pub linked_identities: Vec<OidcLink>,
    pub plan_info: PlanInfo,
}

pub async fn me(
    State(state): State<AppState>,
    AuthedProfile(profile_ulid): AuthedProfile,
) -> ApiResult<Json<MeResponse>> {
    let profile = state.db.profile(&profile_ulid)?;
    let linked = state.db.list_oidc(&profile_ulid)?;
    Ok(Json(MeResponse {
        plan_info: PlanInfo::for_plan(profile.plan),
        profile_ulid: profile.profile_ulid,
        plan: profile.plan,
        created_at: profile.created_at,
        linked_identities: linked,
    }))
}

// ===== POST /v1/account/recover =====

#[derive(Deserialize)]
pub struct RecoverBody {
    pub recovery_code: String,
}

pub async fn recover(
    State(state): State<AppState>,
    Json(body): Json<RecoverBody>,
) -> ApiResult<Json<RegisterResponse>> {
    let profile = state.db.lookup_by_recovery(&body.recovery_code)?;
    let token = mint_token(&profile.profile_ulid, &state.config.jwt_secret, state.config.token_ttl_days)?;
    Ok(Json(RegisterResponse {
        profile_ulid: profile.profile_ulid,
        plan: profile.plan,
        access_token: token,
        created_at: profile.created_at,
    }))
}

// ===== POST /v1/account/oidc/link =====

#[derive(Deserialize)]
pub struct LinkOidcBody {
    pub provider: String,
    pub sub: String,
    pub display_label: Option<String>,
}

pub async fn link_oidc(
    State(state): State<AppState>,
    AuthedProfile(profile_ulid): AuthedProfile,
    Json(body): Json<LinkOidcBody>,
) -> ApiResult<Json<OidcLink>> {
    let link = state.db.link_oidc(
        &profile_ulid,
        &body.provider,
        &body.sub,
        body.display_label.as_deref(),
        &now_iso(),
    )?;
    Ok(Json(link))
}

// ===== DELETE /v1/account/oidc?provider=...&sub=... =====

#[derive(Deserialize)]
pub struct UnlinkParams {
    pub provider: String,
    pub sub: String,
}

pub async fn unlink_oidc(
    State(state): State<AppState>,
    AuthedProfile(profile_ulid): AuthedProfile,
    Query(q): Query<UnlinkParams>,
) -> ApiResult<()> {
    state.db.unlink_oidc(&profile_ulid, &q.provider, &q.sub)?;
    Ok(())
}

// ===== POST /v1/account/oidc/claim =====

#[derive(Deserialize)]
pub struct ClaimOidcBody {
    pub provider: String,
    pub sub: String,
}

pub async fn claim_oidc(
    State(state): State<AppState>,
    Json(body): Json<ClaimOidcBody>,
) -> ApiResult<Json<RegisterResponse>> {
    let profile = state.db.lookup_by_oidc(&body.provider, &body.sub)?;
    let token = mint_token(&profile.profile_ulid, &state.config.jwt_secret, state.config.token_ttl_days)?;
    Ok(Json(RegisterResponse {
        profile_ulid: profile.profile_ulid,
        plan: profile.plan,
        access_token: token,
        created_at: profile.created_at,
    }))
}

// ===== GET /v1/account/plan =====

pub async fn current_plan(
    State(state): State<AppState>,
    AuthedProfile(profile_ulid): AuthedProfile,
) -> ApiResult<Json<PlanInfo>> {
    let profile = state.db.profile(&profile_ulid)?;
    Ok(Json(PlanInfo::for_plan(profile.plan)))
}

// ===== POST /v1/account/plan/checkout =====

#[derive(Serialize)]
pub struct CheckoutResponse {
    pub checkout_url: String,
    pub stub: bool,
}

pub async fn checkout(
    State(_state): State<AppState>,
    AuthedProfile(_profile_ulid): AuthedProfile,
) -> ApiResult<Json<CheckoutResponse>> {
    // Stripe integration lands later; expose a stub URL so the Android
    // upgrade button has somewhere to point.
    Ok(Json(CheckoutResponse {
        checkout_url: "https://ohd.dev/roadmap.html#payments".into(),
        stub: true,
    }))
}

// ===== GET /v1/account/payments =====

pub async fn list_payments(
    State(state): State<AppState>,
    AuthedProfile(profile_ulid): AuthedProfile,
) -> ApiResult<Json<Vec<crate::db::Payment>>> {
    Ok(Json(state.db.list_payments(&profile_ulid)?))
}

// ===== GET /healthz =====

pub async fn healthz() -> &'static str {
    "ok"
}
