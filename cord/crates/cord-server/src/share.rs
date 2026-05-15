//! Parser for the share-link artifact a user hands to CORD.
//!
//! Canonical form (see `cord/spec/data-link.md`):
//!   `ohd://share/<rendezvous_id>?token=<ohdg_…>&pin=<spki>&relay=<host>`
//!
//! The `ohdr://` custom scheme carries the same query string and is
//! accepted as an alias.

use crate::errors::ApiError;
use url::Url;

#[derive(Debug, Clone)]
pub struct ParsedShare {
    pub rendezvous_id: String,
    pub token: String,
    pub pin: Option<String>,
    pub relay_host: String,
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
            "share link must be of the form ohd://share/<rendezvous_id>".into(),
        ));
    }
    let rendezvous_id = u.path().trim_start_matches('/').to_string();
    if rendezvous_id.is_empty() {
        return Err(ApiError::BadRequest(
            "share link is missing the rendezvous id".into(),
        ));
    }
    let (mut token, mut pin, mut relay) = (None, None, None);
    for (k, v) in u.query_pairs() {
        match k.as_ref() {
            "token" => token = Some(v.into_owned()),
            "pin" => pin = Some(v.into_owned()),
            "relay" => relay = Some(v.into_owned()),
            _ => {}
        }
    }
    Ok(ParsedShare {
        rendezvous_id,
        token: token.ok_or_else(|| ApiError::BadRequest("share link is missing `token`".into()))?,
        pin,
        relay_host: relay
            .ok_or_else(|| ApiError::BadRequest("share link is missing `relay`".into()))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_link() {
        let s = parse_share_link(
            "ohd://share/RV123?token=ohdg_abc&pin=PIN9&relay=https://relay.ohd.dev",
        )
        .unwrap();
        assert_eq!(s.rendezvous_id, "RV123");
        assert_eq!(s.token, "ohdg_abc");
        assert_eq!(s.pin.as_deref(), Some("PIN9"));
        assert_eq!(s.relay_host, "https://relay.ohd.dev");
    }

    #[test]
    fn rejects_wrong_scheme() {
        assert!(parse_share_link("https://share/x?token=t&relay=r").is_err());
    }

    #[test]
    fn rejects_missing_token() {
        assert!(parse_share_link("ohd://share/RV?relay=https://relay.ohd.dev").is_err());
    }
}
