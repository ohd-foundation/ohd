//! Reference client for the relay's raw QUIC tunnel mode.
//!
//! # What this is
//!
//! A minimal `quinn` client that:
//!
//! 1. Connects to a relay's `--quic-tunnel-listen` endpoint with ALPN
//!    `ohd-tnl1`.
//! 2. Opens stream 0 (the control / handshake stream).
//! 3. Writes the handshake `[version=1][32-byte credential, NUL-padded]
//!    [u16 token_len BE][token bytes]`.
//! 4. Reads the handshake ack `[ack_status][16 bytes session-base-id]`.
//! 5. Listens for new bidi streams opened by the relay (one per consumer
//!    attach), reads the `[SESSION_OPEN][session_id BE u32]` prefix, and
//!    echoes incoming TunnelFrames back unchanged.
//!
//! # Why this lives here
//!
//! The actual outbound tunnel client belongs in the `ohd-storage-server`
//! binary — that's where the rendezvous-credential lives. This example
//! exists as a wire-shape reference so that integration / can be wired
//! later without re-deriving the framing details.
//!
//! The integration tests in `tests/end_to_end_quic_tunnel.rs` mirror this
//! file's logic for their fake-storage role.
//!
//! # Run
//!
//! ```bash
//! ohd-relay serve --quic-tunnel-listen 127.0.0.1:9001 \
//!     --bind 127.0.0.1:8443 --db /tmp/ohd-relay.db
//!
//! # In another shell, after registering a storage:
//! cargo run --example quic_tunnel_client -- \
//!     --addr 127.0.0.1:9001 \
//!     --rendezvous-id <RID> \
//!     --credential <LLC>
//! ```

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use ohd_relay::frame::TunnelFrame;
use ohd_relay::quic_tunnel::{
    HANDSHAKE_MAX_CRED_LEN, HANDSHAKE_VERSION, STREAM_TAG_SESSION_OPEN, TUNNEL_ALPN,
};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init()
        .ok();

    let mut addr: Option<SocketAddr> = None;
    let mut rid: Option<String> = None;
    let mut cred: Option<String> = None;
    let args: Vec<String> = env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--addr" => {
                addr = Some(args[i + 1].parse()?);
                i += 2;
            }
            "--rendezvous-id" => {
                rid = Some(args[i + 1].clone());
                i += 2;
            }
            "--credential" => {
                cred = Some(args[i + 1].clone());
                i += 2;
            }
            other => {
                anyhow::bail!("unknown flag: {other}");
            }
        }
    }
    let addr = addr.ok_or_else(|| anyhow::anyhow!("--addr required"))?;
    let rid = rid.ok_or_else(|| anyhow::anyhow!("--rendezvous-id required"))?;
    let cred = cred.ok_or_else(|| anyhow::anyhow!("--credential required"))?;

    // Build a quinn client config that accepts any cert (dev relay uses a
    // self-signed cert; production deployments PIN the relay cert via the
    // OHD-Cloud-issued operator chain — out of scope for this example).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut tls = rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(InsecureVerifier {
            provider: rustls::crypto::ring::default_provider(),
        }))
        .with_no_client_auth();
    tls.alpn_protocols = vec![TUNNEL_ALPN.to_vec()];

    let quic_client_cfg: quinn::crypto::rustls::QuicClientConfig =
        tls.try_into().context("ClientConfig → QuicClientConfig")?;
    let client_cfg = quinn::ClientConfig::new(Arc::new(quic_client_cfg));

    let mut endpoint =
        quinn::Endpoint::client("0.0.0.0:0".parse()?).context("quinn::Endpoint::client")?;
    endpoint.set_default_client_config(client_cfg);

    let conn = endpoint
        .connect(addr, "localhost")
        .context("connect")?
        .await
        .context("handshake")?;
    println!("connected: {}", conn.remote_address());

    // Open the handshake stream.
    let (mut send, mut recv) = conn.open_bi().await.context("open_bi")?;
    let mut prefix = vec![HANDSHAKE_VERSION];
    let cred_bytes = cred.as_bytes();
    if cred_bytes.len() > HANDSHAKE_MAX_CRED_LEN {
        anyhow::bail!("credential too long ({} > {HANDSHAKE_MAX_CRED_LEN})", cred_bytes.len());
    }
    prefix.push(cred_bytes.len() as u8);
    prefix.extend_from_slice(cred_bytes);
    let token = rid.as_bytes();
    let token_len = token.len() as u16;
    prefix.extend_from_slice(&token_len.to_be_bytes());
    prefix.extend_from_slice(token);
    send.write_all(&prefix).await.context("write handshake")?;

    // Read ack.
    let mut ack = [0u8; 1 + 16];
    recv.read_exact(&mut ack).await.context("read ack")?;
    if ack[0] != 0 {
        anyhow::bail!("handshake rejected by relay");
    }
    println!("handshake accepted; session-base-id={}", hex::encode(&ack[1..]));

    // Spawn a task that handles new per-session streams: the relay opens a
    // bidi stream for each consumer attach, writes a SESSION_OPEN prefix +
    // an OPEN envelope, then forwards consumer DATA frames. We echo
    // everything back as DATA frames, the simplest possible "storage."
    let conn_for_accept = conn.clone();
    tokio::spawn(async move {
        loop {
            match conn_for_accept.accept_bi().await {
                Ok((send, recv)) => {
                    tokio::spawn(handle_session(send, recv));
                }
                Err(err) => {
                    eprintln!("accept_bi err: {err}");
                    break;
                }
            }
        }
    });

    // Periodic application-level heartbeat on the control stream so the
    // relay's watchdog stays happy.
    let mut tick = tokio::time::interval(Duration::from_secs(30));
    loop {
        tick.tick().await;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut buf = [0u8; 9];
        buf[0] = 0x02; // CONTROL_TAG_HEARTBEAT
        buf[1..].copy_from_slice(&now_ms.to_be_bytes());
        if send.write_all(&buf).await.is_err() {
            eprintln!("control write failed; exiting");
            break;
        }
    }

    Ok(())
}

async fn handle_session(mut send: quinn::SendStream, mut recv: quinn::RecvStream) {
    use bytes::BytesMut;

    let mut prefix = [0u8; 5];
    if recv.read_exact(&mut prefix).await.is_err() {
        return;
    }
    if prefix[0] != STREAM_TAG_SESSION_OPEN {
        eprintln!("unexpected session prefix tag: {}", prefix[0]);
        return;
    }
    let session_id = u32::from_be_bytes([prefix[1], prefix[2], prefix[3], prefix[4]]);
    println!("session opened: id={session_id}");

    // Read & decode TunnelFrames; echo DATA frames straight back.
    let mut buf = BytesMut::new();
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        let Ok(Some(n)) = recv.read(&mut chunk).await else {
            break;
        };
        buf.extend_from_slice(&chunk[..n]);
        loop {
            match TunnelFrame::decode_one(&buf) {
                Ok((frame, consumed)) => {
                    let _ = buf.split_to(consumed);
                    if matches!(frame.frame_type, ohd_relay::frame::FrameType::Open) {
                        // Reply with OPEN_ACK so the relay clears the open.
                        let ack = TunnelFrame::open_ack(session_id).encode().unwrap();
                        let _ = send.write_all(&ack).await;
                    } else if matches!(frame.frame_type, ohd_relay::frame::FrameType::Data) {
                        // Echo back.
                        let echo = TunnelFrame::data(session_id, frame.payload)
                            .encode()
                            .unwrap();
                        let _ = send.write_all(&echo).await;
                    } else if matches!(frame.frame_type, ohd_relay::frame::FrameType::Close) {
                        let _ = send.finish();
                        return;
                    }
                }
                Err(ohd_relay::frame::FrameError::Truncated { .. }) => break,
                Err(err) => {
                    eprintln!("decode err: {err}");
                    return;
                }
            }
        }
    }
}

#[derive(Debug)]
struct InsecureVerifier {
    provider: rustls::crypto::CryptoProvider,
}

impl rustls::client::danger::ServerCertVerifier for InsecureVerifier {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
