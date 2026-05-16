//! # ohd-relay-client
//!
//! Reusable client for the OHD Relay. Speaks the storage side of the relay
//! protocol documented in `relay/spec/relay-protocol.md`:
//!
//! - [`registration`] — `POST /v1/register|heartbeat|deregister`, the
//!   registration HTTP flow that yields a `rendezvous_id` +
//!   `long_lived_credential`.
//! - [`tunnel`] — the `OpenTunnel` QUIC client: dial the relay's
//!   `--quic-tunnel-listen` endpoint (ALPN `ohd-tnl1`), run the handshake,
//!   pulse heartbeats, and demux server-initiated per-session streams to a
//!   pluggable [`tunnel::SessionHandler`].
//! - [`frame`] — the client-side `TunnelFrame` encode/decode codec.
//! - [`tls`] — the three TLS verifier modes (insecure-dev / SPKI-pin /
//!   platform-trust) for the QUIC handshake.
//!
//! # Portability
//!
//! The crate compiles for the Android targets (consumed via the
//! `ohd-storage-bindings` uniffi binding) and for CORD, not just the
//! storage server binary. The default build pulls only portable deps
//! (quinn + rustls + reqwest, all pure-Rust / ring-backed). The hyper
//! HTTP/2 bridge that demuxes relay sessions onto a local
//! `connectrpc::ConnectRpcService` is server-only and lives behind the
//! `tunnel-service` feature; the Android binding builds without it and
//! supplies its own [`tunnel::SessionHandler`].

pub mod frame;
pub mod registration;
pub mod tls;
pub mod tunnel;

#[cfg(feature = "tunnel-service")]
pub mod service;

/// The phone-side share responder — registers a per-share rendezvous,
/// maintains the relay tunnel, terminates the inner TLS server side with
/// the storage identity cert, and speaks share-scoped MCP. Behind the
/// `responder` feature (`ohd-h3-helpers` + `ohd-mcp-core` deps).
#[cfg(feature = "responder")]
pub mod responder;

// -- Re-exports for the common surface --

pub use frame::{decode_one_frame, encode_frame, Frame, FrameError, FrameType};
pub use registration::{
    CredentialedRequest, OkResponse, PushToken, RegisterRequest, RegisterResponse,
    RegistrationClient, RegistrationError,
};
pub use tls::{InsecureCertVerifier, SpkiPinVerifier};
pub use tunnel::{
    serve_relay_tunnel as serve_relay_tunnel_with_handler, AcceptedSession, RelayClientOptions,
    SessionHandler, TUNNEL_ALPN,
};

/// Server-only convenience: run the tunnel client bridging accepted
/// sessions onto a `connectrpc::ConnectRpcService`. Preserves the
/// pre-extraction `serve_relay_tunnel(opts, service, shutdown)` shape.
#[cfg(feature = "tunnel-service")]
pub use service::{serve_relay_tunnel, ConnectRpcSessionHandler};

/// The share responder surface — the phone-side scoped-MCP endpoint.
#[cfg(feature = "responder")]
pub use responder::{register_share_rendezvous, ShareRendezvous, ShareResponder};
