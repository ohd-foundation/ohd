//! Minimal Rekor v1 client for transparency-log entries.
//!
//! Per `spec/emergency-trust.md` "Transparency log (Rekor)":
//!
//! > Every cert issued by OHD-operated Fulcios is logged in OHD-operated
//! > Rekor. Public, append-only, signed, timestamped.
//! >
//! > For v1, transparency-log inclusion proofs are **optional** on the
//! > patient-phone side — too much overhead for the rare emergency event.
//! > The log exists; OHD project + auditors check it.
//!
//! So: we submit an entry per refreshed cert, log the returned `logIndex`
//! so operators can audit, and we treat upload failure as a soft warning
//! (don't block cert use).
//!
//! Wire shape (Rekor v1 `intoto` entry):
//!
//! ```text
//! POST /api/v1/log/entries
//! Content-Type: application/json
//!
//! {
//!   "apiVersion": "0.0.1",
//!   "kind": "intoto",
//!   "spec": {
//!     "content": { "envelope": "<base64 attestation>" },
//!     "publicKey": "<base64 PEM cert>"
//!   }
//! }
//! ```
//!
//! For v1 we wrap the issued cert PEM as the attestation; the canonical
//! Sigstore "hashedrekord" type would be slightly cleaner but requires a
//! separate signature artifact. The `intoto` shape keeps the uploaded
//! payload self-describing and parseable by stock Rekor tooling.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::AuthorityError;

#[derive(Debug, Clone)]
pub struct RekorConfig {
    pub rekor_url: String,
    pub override_entries_url: Option<String>,
    /// Soft-fail mode (default true): log warnings on submission failure
    /// but don't return an error to the caller. Auditing-purpose only;
    /// we don't want a Rekor outage to break authority refresh.
    pub soft_fail: bool,
}

impl RekorConfig {
    fn entries_url(&self) -> String {
        if let Some(o) = &self.override_entries_url {
            return o.clone();
        }
        format!("{}/api/v1/log/entries", self.rekor_url.trim_end_matches('/'))
    }
}

#[derive(Clone)]
pub struct RekorClient {
    cfg: Arc<RekorConfig>,
    http: HttpClient,
}

impl RekorClient {
    pub fn new(cfg: RekorConfig) -> Result<Self, AuthorityError> {
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| AuthorityError::Rekor(format!("build http: {e}")))?;
        Ok(Self {
            cfg: Arc::new(cfg),
            http,
        })
    }

    pub fn with_http(cfg: RekorConfig, http: HttpClient) -> Self {
        Self {
            cfg: Arc::new(cfg),
            http,
        }
    }

    /// Submit a cert PEM to the log. Returns the `logIndex` on success;
    /// `None` if soft-fail is enabled and the submission failed.
    pub async fn submit_cert(&self, cert_pem: &[u8]) -> Result<Option<u64>, AuthorityError> {
        let envelope = base64::engine::general_purpose::STANDARD.encode(cert_pem);
        let pubkey = envelope.clone(); // Rekor expects the public key field; for cert entries it's the cert itself.
        let entry = RekorEntry {
            api_version: "0.0.1".into(),
            kind: "intoto".into(),
            spec: RekorSpec {
                content: RekorContent { envelope },
                public_key: pubkey,
            },
        };
        let url = self.cfg.entries_url();
        let resp = self
            .http
            .post(&url)
            .json(&entry)
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let parsed: RekorEntryResponse = match r.json().await {
                    Ok(p) => p,
                    Err(e) => {
                        if self.cfg.soft_fail {
                            warn!(
                                target: "ohd_relay::rekor",
                                error = %e,
                                "rekor response parse failed (soft-fail)"
                            );
                            return Ok(None);
                        }
                        return Err(AuthorityError::Rekor(format!("parse: {e}")));
                    }
                };
                Ok(Some(parsed.log_index))
            }
            Ok(r) => {
                let status = r.status().as_u16();
                let text = r.text().await.unwrap_or_default();
                if self.cfg.soft_fail {
                    warn!(
                        target: "ohd_relay::rekor",
                        status,
                        body = %text,
                        "rekor submission failed (soft-fail)"
                    );
                    Ok(None)
                } else {
                    Err(AuthorityError::Rekor(format!("{status}: {text}")))
                }
            }
            Err(e) => {
                if self.cfg.soft_fail {
                    warn!(
                        target: "ohd_relay::rekor",
                        error = %e,
                        "rekor request failed (soft-fail)"
                    );
                    Ok(None)
                } else {
                    Err(AuthorityError::Rekor(format!("post: {e}")))
                }
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RekorEntry {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    pub spec: RekorSpec,
}

#[derive(Debug, Serialize)]
pub struct RekorSpec {
    pub content: RekorContent,
    #[serde(rename = "publicKey")]
    pub public_key: String,
}

#[derive(Debug, Serialize)]
pub struct RekorContent {
    pub envelope: String,
}

#[derive(Debug, Deserialize)]
struct RekorEntryResponse {
    #[serde(rename = "logIndex")]
    log_index: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_url_uses_override() {
        let cfg = RekorConfig {
            rekor_url: "https://r.example".into(),
            override_entries_url: Some("http://127.0.0.1/x".into()),
            soft_fail: true,
        };
        assert_eq!(cfg.entries_url(), "http://127.0.0.1/x");
    }

    #[test]
    fn entries_url_appends_path() {
        let cfg = RekorConfig {
            rekor_url: "https://rekor.example/".into(),
            override_entries_url: None,
            soft_fail: true,
        };
        assert_eq!(cfg.entries_url(), "https://rekor.example/api/v1/log/entries");
    }

    #[test]
    fn entry_serializes_to_intoto_shape() {
        let e = RekorEntry {
            api_version: "0.0.1".into(),
            kind: "intoto".into(),
            spec: RekorSpec {
                content: RekorContent {
                    envelope: "AAA".into(),
                },
                public_key: "BBB".into(),
            },
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["apiVersion"], "0.0.1");
        assert_eq!(v["kind"], "intoto");
        assert_eq!(v["spec"]["content"]["envelope"], "AAA");
        assert_eq!(v["spec"]["publicKey"], "BBB");
    }
}
