//! The relying-party client registry.
//!
//! v1 is a static registry: the `[[client]]` entries from `idp.toml` are
//! loaded into an in-memory map keyed by `client_id`. Phase 2's
//! `/authorize` validates an incoming `client_id` + `redirect_uri`
//! against this registry; RFC 7591 dynamic registration is a later,
//! optional addition.

use crate::config::ClientConfig;
use std::collections::HashMap;
use std::sync::Arc;

/// A single registered relying party, as `/authorize` will consume it.
#[derive(Debug, Clone)]
pub struct RegisteredClient {
    pub client_id: String,
    /// Exact-matched redirect URIs — no wildcards (see SPEC "Security").
    pub redirect_uris: Vec<String>,
    /// `true` for a public (PKCE-only) client with no secret.
    pub public: bool,
    /// The resolved client secret; empty for a public client.
    pub client_secret: String,
}

impl RegisteredClient {
    /// Whether `uri` exactly matches one of this client's registered
    /// redirect URIs.
    pub fn allows_redirect(&self, uri: &str) -> bool {
        self.redirect_uris.iter().any(|u| u == uri)
    }
}

/// An in-memory, lookup-by-`client_id` registry of relying parties.
/// Cloneable (`Arc`-backed) so it can live in the axum app state.
#[derive(Debug, Clone, Default)]
pub struct ClientRegistry {
    clients: Arc<HashMap<String, RegisteredClient>>,
}

impl ClientRegistry {
    /// Build the registry from the resolved `[[client]]` config entries.
    pub fn from_config(clients: &[ClientConfig]) -> Self {
        let map = clients
            .iter()
            .map(|c| {
                (
                    c.id.clone(),
                    RegisteredClient {
                        client_id: c.id.clone(),
                        redirect_uris: c.redirect_uris.clone(),
                        public: c.public,
                        client_secret: c.client_secret.clone(),
                    },
                )
            })
            .collect();
        Self {
            clients: Arc::new(map),
        }
    }

    /// Look up a registered client by its `client_id`.
    pub fn get(&self, client_id: &str) -> Option<&RegisteredClient> {
        self.clients.get(client_id)
    }

    /// Number of registered clients.
    pub fn len(&self) -> usize {
        self.clients.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<ClientConfig> {
        vec![
            ClientConfig {
                id: "cord-web".into(),
                redirect_uris: vec!["https://cord.ohd.dev/v1/auth/callback".into()],
                public: false,
                client_secret: "secret-value".into(),
            },
            ClientConfig {
                id: "connect-web".into(),
                redirect_uris: vec!["https://connect.ohd.dev/auth/callback".into()],
                public: true,
                client_secret: String::new(),
            },
        ]
    }

    #[test]
    fn lookup_returns_the_registered_client() {
        let reg = ClientRegistry::from_config(&sample());
        assert_eq!(reg.len(), 2);

        let cord = reg.get("cord-web").expect("cord-web registered");
        assert!(!cord.public);
        assert_eq!(cord.client_secret, "secret-value");
        assert!(cord.allows_redirect("https://cord.ohd.dev/v1/auth/callback"));
        assert!(!cord.allows_redirect("https://evil.example/callback"));

        let connect = reg.get("connect-web").expect("connect-web registered");
        assert!(connect.public);
        assert!(connect.client_secret.is_empty());
    }

    #[test]
    fn unknown_client_id_is_none() {
        let reg = ClientRegistry::from_config(&sample());
        assert!(reg.get("does-not-exist").is_none());
    }
}
