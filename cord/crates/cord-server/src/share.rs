//! Parser for the share-link artifact a user hands to CORD.
//!
//! Two canonical forms — both `ohd://` (the `ohdr://` scheme is an alias):
//!
//!  - **Relay-mediated** (phone-hosted storage behind NAT):
//!    `ohd://share/<rendezvous_id>?token=<ohdg_…>&pin=<spki>&relay=<host>`
//!    Routes through OHD Relay; the inner TLS handshake pins on `spki`.
//!
//!  - **Cloud-direct** (storage reachable on the public internet, e.g.
//!    OHD Cloud / a self-hosted server with a public Caddy cert):
//!    `ohd://share/cloud?endpoint=<storage_url>&token=<ohdg_…>`
//!    No relay, no SPKI pin — the storage URL terminates standard TLS,
//!    the grantee just speaks OHDC to it directly. The 'third card' the
//!    Connect app produces when the user is on OHD Cloud, so the share
//!    flow finishes end-to-end without trying to host a relay tunnel
//!    against a server-side store.

use crate::errors::ApiError;
use url::Url;

/// One parsed share link. The variant carries everything CORD needs to
/// route the connect flow into the right (relay vs direct) branch.
#[derive(Debug, Clone)]
pub enum ParsedShare {
    /// Relay-mediated share — the historic shape.
    Relay {
        rendezvous_id: String,
        token: String,
        pin: Option<String>,
        relay_host: String,
    },
    /// Cloud-direct share — the storage URL is reachable directly.
    Cloud { endpoint: String, token: String },
}

pub fn parse_share_link(link: &str) -> Result<ParsedShare, ApiError> {
    let u = Url::parse(link.trim())
        .map_err(|e| ApiError::BadRequest(format!("malformed share link: {e}")))?;
    if u.scheme() != "ohd" && u.scheme() != "ohdr" {
        return Err(ApiError::BadRequest(
            "share link must use the ohd:// (or ohdr://) scheme".into(),
        ));
    }
    if u.host_str() != Some("share") {
        return Err(ApiError::BadRequest(
            "share link must be of the form ohd://share/…".into(),
        ));
    }

    // Both variants land here; pick by inspecting the path + query.
    let path = u.path().trim_start_matches('/').to_string();
    let (mut token, mut pin, mut relay, mut endpoint) = (None, None, None, None);
    for (k, v) in u.query_pairs() {
        match k.as_ref() {
            "token" => token = Some(v.into_owned()),
            "pin" => pin = Some(v.into_owned()),
            "relay" => relay = Some(v.into_owned()),
            "endpoint" => endpoint = Some(v.into_owned()),
            _ => {}
        }
    }
    let token =
        token.ok_or_else(|| ApiError::BadRequest("share link is missing `token`".into()))?;

    // Cloud-direct: path = "cloud", carries an `endpoint` query parameter.
    // Either an `endpoint` query OR the magic "cloud" path is sufficient
    // — both being present is the canonical form.
    if path == "cloud" || endpoint.is_some() {
        let endpoint = endpoint
            .ok_or_else(|| ApiError::BadRequest("cloud share link is missing `endpoint`".into()))?;
        return Ok(ParsedShare::Cloud { endpoint, token });
    }

    // Relay-mediated: path = rendezvous_id, requires `relay`.
    if path.is_empty() {
        return Err(ApiError::BadRequest(
            "share link is missing the rendezvous id".into(),
        ));
    }
    let relay_host =
        relay.ok_or_else(|| ApiError::BadRequest("share link is missing `relay`".into()))?;
    Ok(ParsedShare::Relay {
        rendezvous_id: path,
        token,
        pin,
        relay_host,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_relay_link() {
        let s = parse_share_link(
            "ohd://share/RV123?token=ohdg_abc&pin=PIN9&relay=https://relay.ohd.dev",
        )
        .unwrap();
        match s {
            ParsedShare::Relay {
                rendezvous_id,
                token,
                pin,
                relay_host,
            } => {
                assert_eq!(rendezvous_id, "RV123");
                assert_eq!(token, "ohdg_abc");
                assert_eq!(pin.as_deref(), Some("PIN9"));
                assert_eq!(relay_host, "https://relay.ohd.dev");
            }
            _ => panic!("expected Relay variant"),
        }
    }

    #[test]
    fn parses_cloud_link() {
        let s = parse_share_link(
            "ohd://share/cloud?endpoint=https%3A%2F%2Fstorage.ohd.dev&token=ohdg_xyz",
        )
        .unwrap();
        match s {
            ParsedShare::Cloud { endpoint, token } => {
                assert_eq!(endpoint, "https://storage.ohd.dev");
                assert_eq!(token, "ohdg_xyz");
            }
            _ => panic!("expected Cloud variant"),
        }
    }

    #[test]
    fn rejects_wrong_scheme() {
        assert!(parse_share_link("https://share/x?token=t&relay=r").is_err());
    }

    #[test]
    fn rejects_missing_token() {
        assert!(parse_share_link("ohd://share/RV?relay=https://relay.ohd.dev").is_err());
    }

    #[test]
    fn rejects_relay_without_host() {
        assert!(parse_share_link("ohd://share/RV?token=t").is_err());
    }

    #[test]
    fn rejects_cloud_without_endpoint() {
        assert!(parse_share_link("ohd://share/cloud?token=t").is_err());
    }
}
