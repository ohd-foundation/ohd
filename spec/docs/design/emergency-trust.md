# Design: Emergency Trust — Authority Certs, Signed Requests

> The cryptographic trust mechanism that makes break-glass safe. Without this, anyone could send an emergency-access request to anyone's phone and (via the timeout-default-allow path) silently get access. With this, the patient phone verifies that a real, known emergency-services authority is asking — institutionally, not individually.
>
> Built on **Sigstore-style short-lived issued certs** (Fulcio for issuance, Rekor for transparency) over **standard X.509 with Ed25519 keys** — no custom OHD-only crypto stack, no revocation lists in normal operation.

## Why we picked this stack

OHD's needs:

- Patient phones verify emergency requests offline-tolerantly.
- Emergency authorities (EMS, hospitals) get cert refreshes that are operationally cheap.
- Compromise window must be tightly bounded.
- Integrators (semi-custom EMS systems, hospital PKI teams) shouldn't have to learn an OHD-only protocol.
- Long-term governance must be transparent and auditable.

Existing open infrastructure that fits:

| Component | What it is | Fit |
|---|---|---|
| **X.509** (RFC 5280, RFC 8410 for Ed25519) | The universal cert format. 30+ years of tooling, every language has a library. | Universal; integrators never write a custom cert parser. |
| **Fulcio** (Sigstore, Apache 2.0) | A short-lived-cert CA. Authenticates clients via OIDC, issues X.509 certs. Designed exactly for the "no long-lived bearer secret" pattern. | OHD runs its own Fulcio instances (one per trust root). Sigstore's hosted Fulcio is a separate concern; we don't depend on it. |
| **Rekor** (Sigstore, Apache 2.0) | Append-only transparency log. Every issued cert is logged; verifiers can prove "this cert was really issued by Fulcio at time T." | OHD runs its own Rekor; alternatively skip in v1, add in v1.x. |
| **Ed25519** (RFC 8032 / 8410) | The signature algorithm. Already used elsewhere in OHD (storage identity key per [`encryption.md`](encryption.md)). | Same crypto throughout the spec. |

We're **consuming** the hard parts (working CA software, transparency log, audited verification libraries) and **contributing** the parts only OHD knows (who counts as an emergency authority, subject naming, governance, deployment topology). The Sigstore project doesn't dictate any of those — they're configurable per deployment.

## What this means for an integrator

An EMS organization integrating their existing system with OHD's emergency network:

1. **Onboard**: their relay's master pubkey is registered with one or more OHD trust roots (one-time human-mediated vetting).
2. **Daily refresh**: a small daemon on their relay calls the OHD Fulcio's `/api/v2/signingCert` endpoint with an OIDC bearer token, gets a fresh 24h X.509 cert. ~50 lines of code in any language.
3. **Sign emergency requests**: standard X.509-with-Ed25519 signing using whatever stock library they already use (`cryptography` in Python, `rustls`/`ring` in Rust, BoringSSL in C++, etc.).

No OHD-specific cert parser. No custom Protobuf canonical encoding. No bespoke verification algorithm. The integrator's emergency-side toolchain is the same one their PKI team already understands.

## Trust hierarchy

```
OHD Project Root CA           (10-year cert, offline HSM, ceremony-rotated)
        │
        ├── OHD Global Fulcio                  (1-year intermediate, online)
        ├── Czech EMS Federation Fulcio        (1-year intermediate, online, country-specific)
        ├── German EMS Federation Fulcio       (1-year intermediate, online)
        └── ...
                │
                ▼
              Org cert  (24h, issued by a Fulcio to an OIDC-authenticated org)
                │
                ▼
              Responder cert  (1-4h, optional, issued by org's internal cert factory)
                │
                ▼
              Emergency-Access Request signed by leaf
```

Up to 4 chain levels typically. Three levels (Root → Fulcio Intermediate → Org cert) is the common case; the responder layer is optional and recommended for orgs that want per-shift accountability.

### Cross-signing

An org can hold multiple parents simultaneously. EMS Prague onboards with both:

- Czech EMS Federation Fulcio (primary)
- OHD Global Fulcio (fallback)

Their relay refreshes from primary; falls back to secondary if primary is unreachable. Both chains terminate at the OHD Project Root, so patient phones validate either equally — no client-side configuration required.

For high availability, the org's relay holds two cert chains; the cross-signing is purely operational (org has two onboarding registrations; daily refresh tries primary first).

### Why up to 4 levels (not arbitrary depth)

- **Root** is offline; can't be reached for daily issuance.
- **Intermediate Fulcio** is OHD's online CA — issues to orgs.
- **Org cert** is the cert the org's relay uses to sign emergency requests.
- **Responder cert** (optional) ties to a specific clinician on shift.

Patient phones cap chain validation at 4 (configurable but the default). Anything deeper is rejected as `INVALID_CHAIN_DEPTH` — pragmatic limit that catches malicious chain-bombing and weird configurations.

## Cert format

Standard X.509 v3 certs with Ed25519 keys (RFC 8410). Specifically:

- `algorithm`: `id-Ed25519` (OID 1.3.101.112)
- `subject`: `CN=<org label>, O=<org name>, C=<country code>, OU=<role>` — conventional X.500 DN
- `issuer`: the parent cert's subject
- `notBefore` / `notAfter`: the validity window
- `serialNumber`: random per cert
- `extensions`:
  - `subjectAltName`: rfc822Name for OIDC subject (for Fulcio-style "this OIDC identity got this cert")
  - `extendedKeyUsage`: includes a custom OID `1.3.6.1.4.1.<OHD-IANA>.1.1` ("OHD emergency authority")
  - `basicConstraints`: `CA:TRUE` for intermediate certs, `CA:FALSE` for leaf
  - `pathLenConstraint`: limits chain depth from this point

We register a private enterprise number with IANA for the OHD-specific OIDs. Until issued, use a placeholder under the `1.3.6.1.4.1.<TBD>` arc; the conformance corpus will pin the value.

## Org refresh flow

Each OHD-operated Fulcio instance exposes the standard Fulcio API:

```
POST /api/v2/signingCert
Authorization: Bearer <OIDC token from OHD's emergency-authority OIDC provider>
Content-Type: application/json

{
  "credentials": {
    "oidcIdentityToken": "<JWT from OIDC provider>"
  },
  "publicKeyRequest": {
    "publicKey": {
      "algorithm": "ED25519",
      "content": "<base64 SPKI of org's daily-refresh keypair>"
    },
    "proofOfPossession": "<Ed25519 signature over OIDC token's email claim>"
  }
}

→ 201 Created
{
  "signedCertificateEmbeddedSct": {
    "chain": {
      "certificates": ["<PEM of org cert>", "<PEM of intermediate>"]
    }
  }
}
```

This is **the standard Fulcio API** — see https://github.com/sigstore/fulcio. Integrators don't write this client; they use Sigstore's existing client libraries (`sigstore-go`, `sigstore-python`, `sigstore-rs`, `cosign`, etc.).

The cert's `notAfter` is set by Fulcio's deployment config (default for OHD: 24 hours). The OHD-deployment Fulcio is configured for:

- Longer cert TTL than Sigstore's hosted Fulcio (10 min for code-signing → 24h for emergency authorities, balancing compromise-window vs refresh-frequency).
- OIDC provider whitelist: only OHD's emergency-authority OIDC IdP is accepted (not arbitrary OIDC sources).
- Subject conventions enforced (CN + O + C required; specific OU values).
- Extended key usage MUST include the OHD emergency-authority OID.

OIDC tokens are issued by an OHD-run OIDC provider whose only purpose is to authenticate emergency-authority orgs. This provider is itself a hardened service, separate from the user-facing OIDC (Google / Apple / OHD Account). Org credentials live in their HSM; the OIDC server attests when they prove possession.

## Per-responder cert (optional, recommended)

For orgs that want individual paramedic accountability:

1. Responder logs in to their org's relay at shift-in (operator OIDC — same one Care uses).
2. Org's relay issues a responder cert (X.509, ~1-4h validity) signed by the org's daily cert.
3. Responder's tablet uses the responder cert to sign emergency-access requests.

Result: the cert chain a patient phone receives includes the responder layer:

```
chain = [responder_cert, org_cert, fulcio_intermediate, ohd_root]
```

When the patient phone verifies and renders the dialog, the audit row records the responder's identity (from the responder cert's subject CN) — sharper than just "EMS Prague Region accessed your data." The patient sees which paramedic in particular.

Post-shift, responder cert expires; off-shift attempts fail. Per-shift = per-cert; clean accountability boundary.

Orgs that don't need this layer skip it; the chain is just `[org_cert, fulcio_intermediate, ohd_root]`. Patient phones accept either depth.

## Signed emergency-access request

The packet that flies from the responder's relay to the patient's storage:

```protobuf
syntax = "proto3";
package ohdc.v1.emergency;

message EmergencyAccessRequest {
  // Anti-replay
  bytes  request_id     = 1;        // 16 random bytes
  int64  issued_at_ms   = 2;
  int64  expires_at_ms  = 3;        // typically issued + 5 minutes

  // Targeting
  bytes  patient_storage_pubkey_pin = 4;  // optional: storage's identity SPKI; misroute fails closed

  // Who & why (informational, surfaced in dialog)
  string responder_label   = 5;     // "Officer Novák", optional; usually mirrored from leaf cert CN
  string scene_context     = 6;     // "medical emergency at Václavské nám." (truncated for safety)
  string operator_label    = 7;     // mirrored from org cert CN; redundant but explicit

  optional double scene_lat = 8;
  optional double scene_lon = 9;
  optional float  scene_accuracy_m = 10;

  // Trust chain — array of PEM-encoded X.509 certs, leaf first, root LAST.
  // Patient phone may already have the root in trusted_authorities and not need it
  // in the chain; the responder's relay sends the full chain anyway for portability.
  repeated bytes cert_chain_pem = 11;

  // Signature by the leaf cert's private key over canonical(this msg without signature).
  bytes  signature = 12;
}
```

PEM-encoded X.509 in the chain (rather than embedding raw DER bytes) so the wire form is grep-able / copy-pasteable from a debug session. Trivial parse cost; matches `openssl x509 -in <file> -text` output people are used to.

The signature is **standard PKCS#1-style detached signature**: Ed25519 over the SHA-512 hash of the canonical Protobuf encoding of the request with `signature = empty bytes`. Standard tooling can produce and verify these.

## Verification algorithm on the patient phone

Pseudocode using standard X.509 path validation:

```
1. Sanity:
   - len(request_id) == 16
   - issued_at_ms in [now - 60s, now + 60s]   # clock skew
   - expires_at_ms > now
   - request_id not in last 24h's seen requests

2. Pin check (if set):
   - patient_storage_pubkey_pin == own_storage_pubkey

3. Chain parse:
   - Decode each PEM cert in cert_chain_pem
   - Build a chain leaf → ... → root

4. X.509 path validation (RFC 5280 standard library call):
   - All certs valid at `now`
   - Each parent's pubkey verifies child's signature
   - Chain terminates at a cert whose pubkey is in trusted_authorities (where removed_at_ms IS NULL)
   - All certs in the chain include the OHD emergency-authority extended-key-usage OID
   - Chain depth ≤ 4

5. Request signature:
   - Verify signature against leaf cert's pubkey over canonical(request without signature)

6. Render dialog:
   - Authority label = leaf cert's subject CN (or org_cert's if responder cert is leaf)
   - Country = leaf cert's subject C
   - Responder = leaf cert's subject CN if it's a responder-level cert
   - Countdown per `_meta.emergency_timeout_s`

7. On grant: clone emergency-template, open case, bind grant via grant_cases, audit.
```

Step 4 is **just X.509 path validation** — every language's TLS / crypto library has a function for this. Implementer doesn't write a verifier; they call `x509::verify_chain(...)` (Rust), `cryptography.x509.verification` (Python), `crypto/x509.Verify` (Go), etc.

The OHD-specific bits are: the trusted-roots list (OHD-specific), the EKU OID (OHD-specific), the depth limit (OHD-specific). Everything else is RFC 5280.

## Trust root storage

```sql
CREATE TABLE trusted_authorities (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  cert_pem        TEXT NOT NULL,                   -- the trust root's cert (X.509 PEM)
  pubkey_sha256   BLOB NOT NULL UNIQUE,            -- for fast lookup / display
  label           TEXT NOT NULL,                   -- "OHD Project Root", "Czech EMS Federation", etc.
  scope_country   TEXT,                            -- ISO 3166-1 alpha-2; NULL = global
  added_at_ms     INTEGER NOT NULL,
  added_by        TEXT NOT NULL,                   -- 'default' | 'user' | 'sync'
  removed_at_ms   INTEGER
);

CREATE INDEX idx_trusted_active ON trusted_authorities (pubkey_sha256) WHERE removed_at_ms IS NULL;
```

Per-user file (under SQLCipher), not system DB — trust-root choices sync across the user's devices.

OHD ships with default roots embedded in the storage build:

1. **OHD Project Root** — operated by the project, signs intermediate Fulcios (regional / sectoral / global).
2. **Country-specific roots** when `_meta.user_country` matches a known country root.

User can remove defaults (with warning) or add custom roots (paste PEM, fingerprint shown for cross-check).

## Revocation — what's the lever, and when?

In normal operation: **none needed**. 24h cert TTL means a compromise window is bounded. Stop issuing fresh certs to a compromised org; within 24h all in-the-wild certs expire; the org generates a new keypair, gets fresh certs, resumes.

For the rare "we need the cert dead *right now*" case:

- **Standard X.509 OCSP / CRL Distribution Points** are supported in the cert format but **not used in OHD's normal operation**. We don't publish CRLs by default.
- For emergency revocation (compromise discovered, can't wait for natural expiry): trust roots can publish a tiny "deny list" of pubkeys to deny pre-expiry. Format and distribution endpoint TBD if this lever is ever needed; v1 doesn't ship it as primary infrastructure. Realistically, "stop issuing new certs and wait 24h" is the answer 99% of the time.

This is the same approach Sigstore uses — they explicitly move away from CRL/OCSP because short-lived certs make them unnecessary in normal operation.

## Transparency log (Rekor)

Every cert issued by OHD-operated Fulcios is logged in OHD-operated Rekor. Public, append-only, signed, timestamped.

Why: a malicious or compromised Fulcio could issue a cert to the wrong party. The transparency log gives every observer (including the patient phone, optionally) the ability to prove "this cert was logged at time T by Fulcio X." A Fulcio that issues secret unlogged certs is detectable post-hoc; a malicious cert presented to a phone but never logged is rejectable.

For v1, transparency-log inclusion proofs are **optional** on the patient-phone side — too much overhead for the rare emergency event. The log exists; OHD project + auditors check it. If a patient is paranoid, a future setting can require inclusion-proof verification per request.

For v1.x: opt-in inclusion proofs as a setting, defaulting off.

## What OHD actually deploys

For the OHD project to operate this:

1. **Root CA**: keypair generated in HSM at a key-ceremony; cert valid 10 years. Offline; only used to sign intermediate Fulcio CSRs, rarely.
2. **Multiple Fulcio instances** (regional + global), each with an intermediate cert from the root. Hosted on production infrastructure (Hetzner / cloud, multi-region, monitored).
3. **Rekor instance**: append-only log, mirrored across regions, cryptographically chained.
4. **OIDC provider** for emergency-authority orgs: hardened, narrow-purpose, separate from user-facing OIDC.
5. **Onboarding workflow**: humans review and approve each org's application. After approval, org's master pubkey is registered as an authorized OIDC client. Daily refreshes are automated thereafter.

Estimated operational cost for the OHD project at v1 scale (hundreds of authorities globally): ~€50-100/month in cloud infrastructure. Sigstore's hosted instances run at much higher scale on similar infrastructure; OHD's traffic is a tiny fraction.

## OHDC RPCs

Emergency-trust-related RPCs added to `OhdcService`:

```protobuf
rpc TrustRootList(TrustRootListRequest) returns (TrustRootListResponse);
rpc TrustRootAdd(TrustRootAddRequest) returns (TrustRootAddResponse);
rpc TrustRootRemove(TrustRootRemoveRequest) returns (TrustRootRemoveResponse);

// The break-glass entry point. No auth header — the request body's cert chain
// is the auth. Any caller (responder relay, BLE-bystander chain, direct internet)
// can invoke; verification rejects unauthorized requests.
rpc DeliverEmergencyRequest(EmergencyAccessRequest) returns (EmergencyAccessResponse);
```

Self-session for the trust-root CRUD; no auth for `DeliverEmergencyRequest` (auth is in the body).

## Cross-references

- Break-glass flow narrative + emergency-template grant: [`storage-format.md`](storage-format.md) "Emergency access"
- Patient-side break-glass UX: [`../../design/screens-emergency.md`](../../design/screens-emergency.md)
- OHD Emergency app component: [`../components/emergency.md`](../components/emergency.md)
- Encryption / Ed25519 storage identity (same crypto used here): [`encryption.md`](encryption.md)
- OHDC general protocol: [`ohdc-protocol.md`](ohdc-protocol.md)
- Notifications (emergency triggers): [`notifications.md`](notifications.md)
- Conformance corpus (cert verification fixtures): [`conformance.md`](conformance.md)

## Open items (deferred to v1.x or beyond)

- **Emergency revocation deny-list** — for the rare "kill cert now" case. Mechanism design + distribution endpoint when first incident requires it. Until then, 24h TTL is the lever.
- **Patient-side Rekor inclusion-proof verification** — opt-in setting that requires every emergency request's cert to have a verifiable transparency-log entry. Adds latency; off by default.
- **Country-CA federation governance** — the formal mechanism by which country-specific emergency-services authorities become trust roots (or sub-issuers under OHD root). Per-country arrangements.
- **Multi-recipient emergency requests** — for mass casualty events where one responder's request might address multiple patients. Not v1.
- **Emergency request transport over BLE** — concrete BLE service UUID, characteristics, and packet framing for the bystander-mediated chain. Implementation-level; deferred to integration phase.
