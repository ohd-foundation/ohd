//! `cert` subcommand bodies.
//!
//! - `cert info` — read the operator's authority cert (PEM at the path in
//!   `config.toml::authority_cert`) and print issuer / subject / SANs /
//!   validity / SHA-256 fingerprint.
//! - `cert refresh` / `cert rotate` — TBD until the relay's Fulcio
//!   integration lands. Print an informative pointer to `emergency-trust.md`
//!   so a sysadmin running the command knows what's blocked and where the
//!   spec lives.
//!
//! `x509-parser` 0.16 is pure-Rust (no openssl) which keeps the CLI's build
//! footprint small.

use std::fs;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use x509_parser::pem::Pem;
use x509_parser::prelude::*;

use crate::config::Config;

/// Print the operator's authority cert at the configured path. If
/// `config.authority_cert` is unset, surface a clear instruction to set it.
pub fn cmd_info(cfg: Option<&Config>) -> Result<()> {
    let path = cfg
        .and_then(|c| c.authority_cert.as_deref())
        .ok_or_else(|| {
            anyhow!(
                "no authority cert configured. Set `authority_cert = \
                 \"/path/to/operator-authority.pem\"` in \
                 `~/.config/ohd-emergency/config.toml`, or run \
                 `cert info --pem PATH` once that flag lands."
            )
        })?;
    print_cert_pem(path)
}

/// Public so the integration test (and `cert info` once we add a `--pem`
/// override) can call it directly.
pub fn print_cert_pem(path: &Path) -> Result<()> {
    let bytes = fs::read(path)
        .with_context(|| format!("read authority cert at {}", path.display()))?;

    // x509-parser's PEM iterator handles concatenated chains.
    let mut idx = 0_usize;
    let mut found_any = false;
    for pem in Pem::iter_from_buffer(&bytes) {
        let pem = pem.with_context(|| format!("parse PEM block #{idx} in {}", path.display()))?;
        let (_, cert) = X509Certificate::from_der(&pem.contents).with_context(|| {
            format!("decode X.509 in PEM block #{idx} of {}", path.display())
        })?;
        found_any = true;

        if idx == 0 {
            println!("authority-cert path: {}", path.display());
        }
        println!();
        println!("--- cert #{idx} ---");
        println!("subject:        {}", cert.subject());
        println!("issuer:         {}", cert.issuer());
        println!(
            "serial:         {}",
            cert.tbs_certificate.serial.to_str_radix(16)
        );
        println!(
            "not_before:     {}",
            cert.validity().not_before.to_rfc2822().unwrap_or_default()
        );
        println!(
            "not_after:      {}",
            cert.validity().not_after.to_rfc2822().unwrap_or_default()
        );
        if let Ok(Some(sans)) = cert.subject_alternative_name() {
            let names: Vec<String> = sans
                .value
                .general_names
                .iter()
                .map(|gn| format!("{gn:?}"))
                .collect();
            if !names.is_empty() {
                println!("subject_alt:    {}", names.join(", "));
            }
        }
        let fp = sha256(&pem.contents);
        println!("sha256:         {}", hex(&fp));
        idx += 1;
    }

    if !found_any {
        return Err(anyhow!(
            "no PEM-encoded certificates found in {}",
            path.display()
        ));
    }
    Ok(())
}

/// `cert refresh` — TBD until the relay's Fulcio integration lands.
pub fn cmd_refresh() -> Result<()> {
    println!("ohd-emergency: cert refresh — not yet implemented");
    println!();
    println!("This command will trigger the relay's daily Fulcio cert refresh.");
    println!("The relay must be running and configured with an OIDC subject");
    println!("registered with the public Fulcio root (or the operator's");
    println!("private Fulcio).");
    println!();
    println!("Spec: ../spec/emergency-trust.md \"Authority cert\" + \"Refresh\".");
    println!("Tracked in ../STATUS.md \"Cross-cutting → Relay's emergency-authority HTTP API\".");
    Ok(())
}

/// `cert rotate` — TBD until Fulcio integration + key-rotation policy land.
pub fn cmd_rotate() -> Result<()> {
    println!("ohd-emergency: cert rotate — not yet implemented");
    println!();
    println!("This command will rotate the operator's daily-refresh keypair:");
    println!("  1. generate a fresh keypair (HSM-backed where available),");
    println!("  2. register the new public key with Fulcio,");
    println!("  3. drop the old keypair after the grace window.");
    println!();
    println!("Spec: ../spec/emergency-trust.md \"Key rotation\".");
    Ok(())
}

// ---- crypto helpers (no external deps; SHA-256 is in `ring` via rustls
//      transitively but exposing it here would pull more API surface than
//      we need — so a tiny SHA-256 wrapper using the rustls dep instead).

fn sha256(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    let out = h.finalize();
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&out[..]);
    buf
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && i % 2 == 0 {
            s.push(':');
        }
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn mint_self_signed_pem(dir: &Path) -> std::path::PathBuf {
        let names = vec!["test-emergency-authority".to_string()];
        let rcgen::CertifiedKey { cert, key_pair: _ } =
            rcgen::generate_simple_self_signed(names).expect("rcgen");
        let pem = cert.pem();
        let path = dir.join("authority.pem");
        std::fs::write(&path, pem).unwrap();
        path
    }

    #[test]
    fn print_self_signed_cert() {
        let dir = tempdir().unwrap();
        let path = mint_self_signed_pem(dir.path());
        // Smoke: must not panic / error on a valid PEM.
        print_cert_pem(&path).unwrap();
    }

    #[test]
    fn missing_pem_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("none.pem");
        let err = print_cert_pem(&path).unwrap_err();
        assert!(format!("{err:#}").contains("read authority cert"));
    }

    #[test]
    fn empty_pem_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("empty.pem");
        std::fs::write(&path, "").unwrap();
        let err = print_cert_pem(&path).unwrap_err();
        assert!(format!("{err:#}").contains("no PEM-encoded"));
    }
}
