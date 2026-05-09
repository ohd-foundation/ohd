//! Round-trip test for the `/v1/emergency/initiate` endpoint and the
//! `auth_mode::sign_request` / `verify_request` pair.
//!
//! Authority mode requires Fulcio + an OIDC token to mint a real chain;
//! we stand in with a self-signed Ed25519 cert and exercise the
//! sign-then-verify path. The actual `/v1/emergency/initiate` HTTP
//! endpoint is exercised in unit-test form via the handler module rather
//! than over a live socket — booting the full server requires a live
//! Fulcio, which we shouldn't depend on for unit tests.
//!
//! Real-Fulcio integration tests live behind `OHD_FULCIO_URL` +
//! `OHD_FULCIO_OIDC_TOKEN_PATH` env vars (see `tests/end_to_end_authority.rs`
//! when it lands).

#![cfg(feature = "authority")]

use std::time::Duration;

use ed25519_dalek::{Signer, SigningKey};
use ohd_relay::auth_mode::{
    canonical_signing_bytes, sign_request, verify_request, AuthorityCertChain,
    EmergencyAccessRequest, EmergencyTrustError, SignedEmergencyRequest, TrustRoot,
};
use rand::rngs::OsRng;
use rcgen::{CertificateParams, KeyPair};

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn ohd_eku_extension() -> rcgen::CustomExtension {
    let oid_body = [
        0x2B, 0x06, 0x01, 0x04, 0x01, 0x86, 0x8F, 0x1F, 0x01, 0x01,
    ];
    let mut ext_value = Vec::new();
    ext_value.push(0x30);
    ext_value.push((2 + oid_body.len()) as u8);
    ext_value.push(0x06);
    ext_value.push(oid_body.len() as u8);
    ext_value.extend_from_slice(&oid_body);
    let mut ext = rcgen::CustomExtension::from_oid_content(&[2, 5, 29, 37], ext_value);
    ext.set_criticality(false);
    ext
}

fn build_self_signed_chain(validity_secs: u64) -> AuthorityCertChain {
    use ed25519_dalek::pkcs8::EncodePrivateKey;
    let mut csprng = OsRng;
    let signing_key = SigningKey::generate(&mut csprng);
    let pkcs8_der = signing_key.to_pkcs8_der().unwrap().as_bytes().to_vec();
    let kp = KeyPair::try_from(pkcs8_der.as_slice()).unwrap();
    let mut params = CertificateParams::new(vec!["ohd-test".into()]).unwrap();
    let now = std::time::SystemTime::now();
    params.not_before = now.into();
    params.not_after = (now + Duration::from_secs(validity_secs)).into();
    params.custom_extensions.push(ohd_eku_extension());
    let cert = params.self_signed(&kp).unwrap();
    let pem = cert.pem().into_bytes();
    let (nb, na) = AuthorityCertChain::parse_leaf_validity(&pem).unwrap();
    AuthorityCertChain {
        leaf_pem: pem.clone(),
        intermediate_pem: pem.clone(),
        root_pem: pem.clone(),
        leaf_signing_key: signing_key,
        leaf_not_before_ms: nb,
        leaf_not_after_ms: na,
    }
}

#[test]
fn sign_then_verify_emergency_request_roundtrip() {
    let chain = build_self_signed_chain(3600);
    let trust = vec![TrustRoot {
        pem: chain.root_pem.clone(),
        label: "test-root".into(),
    }];
    let req = EmergencyAccessRequest {
        request_id: String::new(),
        issued_at_ms: 0,
        expires_at_ms: 0,
        patient_storage_pubkey_pin: None,
        responder_label: Some("Officer Test".into()),
        scene_context: Some("emergency drill".into()),
        operator_label: Some("EMS Test Region".into()),
        scene_lat: Some(50.0875),
        scene_lon: Some(14.4213),
        scene_accuracy_m: Some(15.0),
        cert_chain_pem: vec![],
    };
    let mut signed = sign_request(&chain, req, now_ms()).unwrap();

    // Collapse the duplicated 3-cert chain to 1 cert (self-signed) so the
    // chain-signature loop doesn't try to verify cert N against cert N+1
    // when both are the same.
    signed.request.cert_chain_pem.truncate(1);
    // Re-sign because canonical bytes changed.
    let digest = canonical_signing_bytes(&signed.request).unwrap();
    let sig = chain.leaf_signing_key.sign(&digest);
    signed.signature = base64_encode(sig.to_bytes().as_ref());

    verify_request(&signed, &trust, now_ms(), None).expect("verify");
}

#[test]
fn verify_rejects_tampered_payload() {
    let chain = build_self_signed_chain(3600);
    let trust = vec![TrustRoot {
        pem: chain.root_pem.clone(),
        label: "test-root".into(),
    }];
    let req = EmergencyAccessRequest {
        request_id: String::new(),
        issued_at_ms: 0,
        expires_at_ms: 0,
        patient_storage_pubkey_pin: None,
        responder_label: Some("Officer Test".into()),
        scene_context: Some("real emergency".into()),
        operator_label: None,
        scene_lat: None,
        scene_lon: None,
        scene_accuracy_m: None,
        cert_chain_pem: vec![],
    };
    let mut signed = sign_request(&chain, req, now_ms()).unwrap();
    signed.request.cert_chain_pem.truncate(1);
    let digest = canonical_signing_bytes(&signed.request).unwrap();
    let sig = chain.leaf_signing_key.sign(&digest);
    signed.signature = base64_encode(sig.to_bytes().as_ref());
    verify_request(&signed, &trust, now_ms(), None).expect("baseline ok");

    // Tamper with the scene_context after signing.
    signed.request.scene_context = Some("ALTERED CONTEXT".into());
    let r = verify_request(&signed, &trust, now_ms(), None);
    assert!(matches!(r, Err(EmergencyTrustError::RequestSignatureMismatch)));
}

#[test]
fn verify_rejects_expired_chain() {
    // Build a cert with very short validity, sleep past expiry.
    let chain = build_self_signed_chain(1);
    let trust = vec![TrustRoot {
        pem: chain.root_pem.clone(),
        label: "test-root".into(),
    }];
    std::thread::sleep(Duration::from_millis(1500));
    let req = EmergencyAccessRequest {
        request_id: String::new(),
        issued_at_ms: 0,
        expires_at_ms: 0,
        patient_storage_pubkey_pin: None,
        responder_label: None,
        scene_context: None,
        operator_label: None,
        scene_lat: None,
        scene_lon: None,
        scene_accuracy_m: None,
        cert_chain_pem: vec![],
    };
    let mut signed = sign_request(&chain, req, now_ms()).unwrap();
    signed.request.cert_chain_pem.truncate(1);
    let digest = canonical_signing_bytes(&signed.request).unwrap();
    let sig = chain.leaf_signing_key.sign(&digest);
    signed.signature = base64_encode(sig.to_bytes().as_ref());

    let r = verify_request(&signed, &trust, now_ms(), None);
    assert!(matches!(r, Err(EmergencyTrustError::ChainValidity(_))));
}

#[test]
fn verify_rejects_pin_mismatch() {
    let chain = build_self_signed_chain(3600);
    let trust = vec![TrustRoot {
        pem: chain.root_pem.clone(),
        label: "test-root".into(),
    }];
    let req = EmergencyAccessRequest {
        request_id: String::new(),
        issued_at_ms: 0,
        expires_at_ms: 0,
        patient_storage_pubkey_pin: Some(hex::encode([0xAAu8; 32])),
        responder_label: None,
        scene_context: None,
        operator_label: None,
        scene_lat: None,
        scene_lon: None,
        scene_accuracy_m: None,
        cert_chain_pem: vec![],
    };
    let mut signed = sign_request(&chain, req, now_ms()).unwrap();
    signed.request.cert_chain_pem.truncate(1);
    let digest = canonical_signing_bytes(&signed.request).unwrap();
    let sig = chain.leaf_signing_key.sign(&digest);
    signed.signature = base64_encode(sig.to_bytes().as_ref());

    // Caller-supplied pin disagrees with what the request claims.
    let actual_storage_pin = [0xBBu8; 32];
    let r = verify_request(&signed, &trust, now_ms(), Some(&actual_storage_pin));
    assert!(matches!(r, Err(EmergencyTrustError::PinMismatch)));
}

fn base64_encode(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}

// Silence unused import warnings when feature is enabled but tests skip.
#[allow(dead_code)]
fn _unused_signed_marker(_: SignedEmergencyRequest) {}
