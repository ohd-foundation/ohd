//! Operator-side responder roster — TOML state, no OHDC.
//!
//! The canonical responder roster lives on the operator's IdP / relay.
//! For v0 the CLI keeps a *local* state file at
//! `$XDG_DATA_HOME/ohd-emergency/roster.toml` (or `config.roster_path`)
//! so sysadmins can add / remove / list responders without spinning up
//! the IdP layer. Once `relay/` exposes a roster API this module gets
//! a network mode; until then it's local TOML.
//!
//! Schema (simple, append-only friendly):
//!
//! ```toml
//! [[responder]]
//! label    = "Dr.Test"
//! role     = "responder"          # responder | dispatcher
//! added_at = "2026-05-08T12:34:56Z"
//! on_duty  = true
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{self, Config};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Roster {
    #[serde(default, rename = "responder")]
    pub entries: Vec<RosterEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterEntry {
    pub label: String,
    pub role: String,
    pub added_at: DateTime<Utc>,
    #[serde(default = "default_on_duty")]
    pub on_duty: bool,
}

fn default_on_duty() -> bool {
    true
}

/// Resolve where the roster TOML lives. CLI flag wins, then `config.toml`,
/// then the default `$XDG_DATA_HOME/ohd-emergency/roster.toml`.
pub fn resolve_path(cli_override: Option<&Path>, cfg: Option<&Config>) -> Result<PathBuf> {
    if let Some(p) = cli_override {
        return Ok(p.to_path_buf());
    }
    if let Some(c) = cfg {
        if let Some(p) = c.roster_path.as_ref() {
            return Ok(p.clone());
        }
    }
    config::default_roster_path()
}

/// Load the roster TOML. If the file is missing, returns a default empty
/// roster. (`list` / `status` should work out-of-the-box on a fresh install.)
pub fn load(path: &Path) -> Result<Roster> {
    if !path.exists() {
        return Ok(Roster::default());
    }
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read roster at {}", path.display()))?;
    let roster: Roster = toml::from_str(&raw)
        .with_context(|| format!("parse roster at {}", path.display()))?;
    Ok(roster)
}

pub fn save(path: &Path, roster: &Roster) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("mkdir {}", parent.display()))?;
    }
    let serialized = toml::to_string_pretty(roster).context("serialize roster")?;
    fs::write(path, serialized).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

// ---- subcommand bodies ----------------------------------------------------

pub fn cmd_list(path: &Path) -> Result<()> {
    let roster = load(path)?;
    if roster.entries.is_empty() {
        println!("no responders in roster ({})", path.display());
        println!("add one with: ohd-emergency roster add --label NAME --role responder");
        return Ok(());
    }
    println!("roster: {} ({} entr{})",
        path.display(),
        roster.entries.len(),
        if roster.entries.len() == 1 { "y" } else { "ies" }
    );
    println!();
    println!("{:<24}  {:<11}  {:<11}  {}", "LABEL", "ROLE", "ON-DUTY", "ADDED");
    for e in &roster.entries {
        println!(
            "{:<24}  {:<11}  {:<11}  {}",
            e.label,
            e.role,
            if e.on_duty { "yes" } else { "no" },
            e.added_at.to_rfc3339()
        );
    }
    Ok(())
}

pub fn cmd_add(path: &Path, label: &str, role: &str) -> Result<()> {
    let role = match role {
        "responder" | "dispatcher" => role.to_string(),
        other => {
            return Err(anyhow!(
                "unknown role {other:?}; expected `responder` or `dispatcher`"
            ));
        }
    };
    let mut roster = load(path)?;
    if roster.entries.iter().any(|e| e.label == label) {
        return Err(anyhow!(
            "responder {label:?} is already in the roster (remove first to re-add)"
        ));
    }
    roster.entries.push(RosterEntry {
        label: label.to_string(),
        role,
        added_at: Utc::now(),
        on_duty: true,
    });
    save(path, &roster)?;
    println!("added {label:?} → {}", path.display());
    Ok(())
}

pub fn cmd_remove(path: &Path, label: &str) -> Result<()> {
    let mut roster = load(path)?;
    let before = roster.entries.len();
    roster.entries.retain(|e| e.label != label);
    if roster.entries.len() == before {
        return Err(anyhow!(
            "responder {label:?} not in roster ({} entr{})",
            roster.entries.len(),
            if roster.entries.len() == 1 { "y" } else { "ies" }
        ));
    }
    save(path, &roster)?;
    println!("removed {label:?} from {}", path.display());
    Ok(())
}

pub fn cmd_status(path: &Path) -> Result<()> {
    let roster = load(path)?;
    let total = roster.entries.len();
    let on_duty = roster.entries.iter().filter(|e| e.on_duty).count();
    let responders = roster
        .entries
        .iter()
        .filter(|e| e.role == "responder")
        .count();
    let dispatchers = roster
        .entries
        .iter()
        .filter(|e| e.role == "dispatcher")
        .count();
    println!("roster:      {}", path.display());
    println!("total:       {total}");
    println!("on-duty:     {on_duty}");
    println!("responders:  {responders}");
    println!("dispatchers: {dispatchers}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn add_then_list_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roster.toml");

        cmd_add(&path, "Dr.Test", "responder").unwrap();
        cmd_add(&path, "Disp.Alice", "dispatcher").unwrap();

        let roster = load(&path).unwrap();
        assert_eq!(roster.entries.len(), 2);
        assert_eq!(roster.entries[0].label, "Dr.Test");
        assert_eq!(roster.entries[0].role, "responder");
        assert!(roster.entries[0].on_duty);
        assert_eq!(roster.entries[1].label, "Disp.Alice");
        assert_eq!(roster.entries[1].role, "dispatcher");
    }

    #[test]
    fn duplicate_add_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roster.toml");
        cmd_add(&path, "X", "responder").unwrap();
        let err = cmd_add(&path, "X", "responder").unwrap_err();
        assert!(format!("{err:#}").contains("already"));
    }

    #[test]
    fn remove_missing_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roster.toml");
        let err = cmd_remove(&path, "Ghost").unwrap_err();
        assert!(format!("{err:#}").contains("not in roster"));
    }

    #[test]
    fn unknown_role_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roster.toml");
        let err = cmd_add(&path, "Y", "wizard").unwrap_err();
        assert!(format!("{err:#}").contains("unknown role"));
    }

    #[test]
    fn list_empty_succeeds() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roster.toml");
        cmd_list(&path).unwrap();
        cmd_status(&path).unwrap();
    }

    #[test]
    fn manual_toml_edit_parses() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roster.toml");
        let raw = r#"
[[responder]]
label = "Dr.Manual"
role = "responder"
added_at = "2026-05-08T00:00:00Z"
on_duty = false
"#;
        std::fs::write(&path, raw).unwrap();
        let roster = load(&path).unwrap();
        assert_eq!(roster.entries.len(), 1);
        assert!(!roster.entries[0].on_duty);
    }
}
