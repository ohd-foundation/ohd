//! End-to-end test of the multi-identity link flow against the in-process
//! `ohd-storage-core` API. Drives:
//!
//!  1. Create a fresh storage file (mints a `user_ulid`).
//!  2. Bootstrap the user's first OIDC identity (issuer_a, sub_a).
//!  3. Verify [`find_user_by_identity`] resolves issuer_a + sub_a → user_ulid.
//!  4. `link_identity_start` returns a `link_token`.
//!  5. Build a mock id_token signed by issuer_b, verify
//!     [`complete_identity_link`] inserts a second `_oidc_identities` row.
//!  6. [`list_identities`] returns 2 rows.
//!  7. Sign-in via issuer_b resolves to the same user_ulid.
//!  8. [`unlink_identity`] removes one. Last-identity unlink returns
//!     `OutOfScope` (the wire surface maps this to LAST_IDENTITY_PROTECTED).

use jsonwebtoken::jwk::{
    AlgorithmParameters, CommonParameters, Jwk, JwkSet, KeyAlgorithm, PublicKeyUse,
    RSAKeyParameters, RSAKeyType,
};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use ohd_storage_core::identities::{self, IssuerVerification, StaticJwksResolver};
use ohd_storage_core::ulid::Ulid;
use ohd_storage_core::{Storage, StorageConfig};
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use serde::Serialize;

/// Minimal OIDC id_token issuer for tests. Generates an RSA key pair, issues
/// `id_tokens` signed RS256, exposes the matching JWK to the resolver.
#[allow(dead_code)]
struct MockIssuer {
    issuer: String,
    audience: String,
    kid: String,
    private_key: RsaPrivateKey,
    encoding_key: EncodingKey,
    jwks: JwkSet,
}

impl MockIssuer {
    fn new(issuer: impl Into<String>, audience: impl Into<String>, kid: impl Into<String>) -> Self {
        let issuer = issuer.into();
        let audience = audience.into();
        let kid = kid.into();

        // Generate a 2048-bit RSA key. 2048 is fine for tests; production
        // identity providers vary between 2048 and 4096.
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("RSA generate");
        let public_key = private_key.to_public_key();

        // Build the JWK from the public-key components (n, e in base64url).
        let n_bytes = public_key.n().to_bytes_be();
        let e_bytes = public_key.e().to_bytes_be();
        use base64::Engine;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;

        let jwk = Jwk {
            common: CommonParameters {
                public_key_use: Some(PublicKeyUse::Signature),
                key_algorithm: Some(KeyAlgorithm::RS256),
                key_id: Some(kid.clone()),
                ..Default::default()
            },
            algorithm: AlgorithmParameters::RSA(RSAKeyParameters {
                key_type: RSAKeyType::RSA,
                n: b64.encode(&n_bytes),
                e: b64.encode(&e_bytes),
            }),
        };
        // Debug print to investigate InvalidAlgorithm.
        // eprintln!("JWK: {}", serde_json::to_string(&jwk).unwrap());
        let jwks = JwkSet { keys: vec![jwk] };

        // Encoding key: PEM-encoded PKCS#1 (jsonwebtoken's `from_rsa_pem`).
        let pem = private_key
            .to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
            .expect("to_pkcs1_pem");
        let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes()).expect("from_rsa_pem");

        Self {
            issuer,
            audience,
            kid,
            private_key,
            encoding_key,
            jwks,
        }
    }

    fn id_token(&self, subject: &str, email: Option<&str>) -> String {
        #[derive(Serialize)]
        struct Claims<'a> {
            iss: &'a str,
            aud: &'a str,
            sub: &'a str,
            iat: i64,
            exp: i64,
            #[serde(skip_serializing_if = "Option::is_none")]
            email: Option<&'a str>,
        }
        let now = ohd_storage_core::format::now_ms() / 1000;
        let claims = Claims {
            iss: &self.issuer,
            aud: &self.audience,
            sub: subject,
            iat: now,
            exp: now + 600,
            email,
        };
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.kid.clone());
        encode(&header, &claims, &self.encoding_key).expect("encode id_token")
    }
}

fn make_user(byte: u8) -> Ulid {
    let mut u = [0u8; 16];
    u[15] = byte;
    u
}

#[test]
fn full_link_flow_two_issuers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ids.db");
    let key: Vec<u8> = (0u8..32).collect();
    let storage = Storage::open(StorageConfig::new(&path).with_cipher_key(key)).expect("open");
    let user_ulid = storage.user_ulid();

    // Set up two mock OIDC issuers with their JWKs.
    let issuer_a = MockIssuer::new("https://accounts.google.test", "ohd-client-a", "kid-a");
    let issuer_b = MockIssuer::new("https://login.facebook.test", "ohd-client-b", "kid-b");

    let resolver = StaticJwksResolver::new();
    resolver.insert(issuer_a.issuer.clone(), issuer_a.jwks.clone());
    resolver.insert(issuer_b.issuer.clone(), issuer_b.jwks.clone());

    // Step 1: bootstrap initial identity (issuer_a, sub_alice).
    storage
        .with_conn(|conn| {
            identities::bootstrap_first_identity(
                conn,
                user_ulid,
                &issuer_a.issuer,
                "sub_alice_google",
                Some("alice@example.com"),
                Some("Personal Google"),
            )
            .map(|_| ())
        })
        .expect("bootstrap A");

    // Sign-in resolution via issuer_a.
    let resolved = storage
        .with_conn(|conn| {
            identities::find_user_by_identity(conn, &issuer_a.issuer, "sub_alice_google")
        })
        .expect("find A")
        .expect("Some");
    assert_eq!(resolved, user_ulid);

    // Step 2: Alice starts linking issuer_b.
    let outcome = storage
        .with_conn(|conn| identities::link_identity_start(conn, user_ulid, None, Some("facebook")))
        .expect("link_start");
    assert!(!outcome.link_token.is_empty());

    // Step 3: Alice's Connect app calls the issuer_b OAuth flow. The provider
    // returns an id_token. Build a mock one.
    let id_token = issuer_b.id_token("sub_alice_facebook", Some("alice@fb.example.com"));

    let cfg = IssuerVerification::new(issuer_b.issuer.clone(), vec![issuer_b.audience.clone()]);

    // Step 4: complete_identity_link.
    let identity = storage
        .with_conn_mut(|conn| {
            identities::complete_identity_link(
                conn,
                &outcome.link_token,
                &id_token,
                &cfg,
                &resolver,
                Some("Personal Facebook"),
            )
        })
        .expect("complete_link");
    assert_eq!(identity.provider, issuer_b.issuer);
    assert_eq!(identity.subject, "sub_alice_facebook");
    assert_eq!(identity.user_ulid, user_ulid);
    assert!(!identity.is_primary, "first identity stays primary");
    assert_eq!(identity.display_label.as_deref(), Some("Personal Facebook"));

    // Step 5: list returns 2 identities.
    let rows = storage
        .with_conn(|conn| identities::list_identities(conn, user_ulid))
        .expect("list");
    assert_eq!(rows.len(), 2);
    // Primary first.
    assert!(rows[0].is_primary);
    assert_eq!(rows[0].provider, issuer_a.issuer);

    // Step 6: sign-in via issuer_b → same user_ulid.
    let resolved_b = storage
        .with_conn(|conn| {
            identities::find_user_by_identity(conn, &issuer_b.issuer, "sub_alice_facebook")
        })
        .expect("find B")
        .expect("Some");
    assert_eq!(resolved_b, user_ulid);

    // Step 7: unlink issuer_b.
    storage
        .with_conn_mut(|conn| {
            identities::unlink_identity(conn, user_ulid, &issuer_b.issuer, "sub_alice_facebook")
        })
        .expect("unlink");

    // Step 8: try to unlink the last (issuer_a) → OutOfScope (LAST_IDENTITY_PROTECTED).
    let res = storage.with_conn_mut(|conn| {
        identities::unlink_identity(conn, user_ulid, &issuer_a.issuer, "sub_alice_google")
    });
    assert!(
        matches!(res, Err(ohd_storage_core::Error::OutOfScope)),
        "last identity must be protected, got {res:?}"
    );

    // Sanity: still has 1 row.
    let rows = storage
        .with_conn(|conn| identities::list_identities(conn, user_ulid))
        .expect("list final");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].provider, issuer_a.issuer);
}

#[test]
fn complete_link_with_invalid_token_fails_no_db_change() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ids.db");
    let storage = Storage::open(StorageConfig::new(&path)).expect("open");
    let user_ulid = storage.user_ulid();

    let issuer = MockIssuer::new("https://test.issuer", "aud-x", "kid-x");
    let resolver = StaticJwksResolver::new();
    resolver.insert(issuer.issuer.clone(), issuer.jwks.clone());

    let outcome = storage
        .with_conn(|conn| identities::link_identity_start(conn, user_ulid, None, None))
        .expect("link_start");

    // Garbled JWT — verification should fail before any DB write.
    let cfg = IssuerVerification::new(issuer.issuer.clone(), vec!["aud-x".into()]);
    let res = storage.with_conn_mut(|conn| {
        identities::complete_identity_link(
            conn,
            &outcome.link_token,
            "this.is.not.a.jwt",
            &cfg,
            &resolver,
            None,
        )
    });
    assert!(matches!(
        res,
        Err(ohd_storage_core::Error::InvalidArgument(_))
    ));

    // The pending row should still be open (not marked completed).
    let count: i64 = storage
        .with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM _pending_identity_links WHERE completed = 0",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(ohd_storage_core::Error::from)
        })
        .expect("count");
    assert_eq!(
        count, 1,
        "pending row should remain open after failed verification"
    );

    // No identities should have been inserted.
    let rows = storage
        .with_conn(|conn| identities::list_identities(conn, user_ulid))
        .expect("list");
    assert_eq!(rows.len(), 0);
}

#[test]
fn link_token_with_wrong_audience_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ids.db");
    let storage = Storage::open(StorageConfig::new(&path)).expect("open");
    let user_ulid = storage.user_ulid();

    let issuer = MockIssuer::new("https://provider.test", "actual-aud", "kid-1");
    let resolver = StaticJwksResolver::new();
    resolver.insert(issuer.issuer.clone(), issuer.jwks.clone());

    let outcome = storage
        .with_conn(|conn| identities::link_identity_start(conn, user_ulid, None, None))
        .expect("link_start");

    let id_token = issuer.id_token("sub-1", None);

    // Pin a *different* audience than the one in the token.
    let cfg = IssuerVerification::new(issuer.issuer.clone(), vec!["wrong-aud".into()]);

    let res = storage.with_conn_mut(|conn| {
        identities::complete_identity_link(
            conn,
            &outcome.link_token,
            &id_token,
            &cfg,
            &resolver,
            None,
        )
    });
    assert!(matches!(
        res,
        Err(ohd_storage_core::Error::InvalidArgument(_))
    ));
}

#[test]
fn double_complete_with_same_link_token_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ids.db");
    let storage = Storage::open(StorageConfig::new(&path)).expect("open");
    let user_ulid = storage.user_ulid();

    let issuer = MockIssuer::new("https://once.test", "aud-y", "kid-y");
    let resolver = StaticJwksResolver::new();
    resolver.insert(issuer.issuer.clone(), issuer.jwks.clone());

    let outcome = storage
        .with_conn(|conn| identities::link_identity_start(conn, user_ulid, None, None))
        .expect("link_start");
    let id_token = issuer.id_token("sub-once", None);
    let cfg = IssuerVerification::new(issuer.issuer.clone(), vec!["aud-y".into()]);

    storage
        .with_conn_mut(|conn| {
            identities::complete_identity_link(
                conn,
                &outcome.link_token,
                &id_token,
                &cfg,
                &resolver,
                None,
            )
        })
        .expect("first complete");

    // Re-using the same link_token should fail.
    let res = storage.with_conn_mut(|conn| {
        identities::complete_identity_link(
            conn,
            &outcome.link_token,
            &id_token,
            &cfg,
            &resolver,
            None,
        )
    });
    assert!(matches!(
        res,
        Err(ohd_storage_core::Error::InvalidArgument(_))
    ));
}

#[test]
fn cross_user_idempotency_conflict() {
    // Two distinct OHD users; user2 attempts to link an identity that's
    // already bound to user1. Must return IdempotencyConflict, not silently
    // overwrite.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ids.db");
    let storage = Storage::open(StorageConfig::new(&path)).expect("open");
    let user1 = storage.user_ulid();
    let user2 = make_user(99);
    assert_ne!(user1, user2);

    let issuer = MockIssuer::new("https://shared.test", "aud", "k");
    let resolver = StaticJwksResolver::new();
    resolver.insert(issuer.issuer.clone(), issuer.jwks.clone());

    // user1 already has the (provider, subject) bound.
    storage
        .with_conn(|conn| {
            identities::bootstrap_first_identity(
                conn,
                user1,
                &issuer.issuer,
                "shared-sub",
                None,
                None,
            )
            .map(|_| ())
        })
        .unwrap();

    // user2 tries to link the same (provider, subject) — fabricate by
    // starting a link as user2.
    let outcome = storage
        .with_conn(|conn| identities::link_identity_start(conn, user2, None, None))
        .expect("link_start");
    let id_token = issuer.id_token("shared-sub", None);
    let cfg = IssuerVerification::new(issuer.issuer.clone(), vec!["aud".into()]);

    let res = storage.with_conn_mut(|conn| {
        identities::complete_identity_link(
            conn,
            &outcome.link_token,
            &id_token,
            &cfg,
            &resolver,
            None,
        )
    });
    assert!(
        matches!(res, Err(ohd_storage_core::Error::IdempotencyConflict)),
        "got: {res:?}"
    );
}
