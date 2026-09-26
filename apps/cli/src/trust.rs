//! Trust and untrust the local CA in the login keychain, with `/usr/bin/security`.
//! The daemon never does this (ADR 01, change 4): macOS shows a password dialog.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};

const SECURITY: &str = "/usr/bin/security";

fn login_keychain() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/Library/Keychains/login.keychain-db")
}

pub fn trust(pem: &Path) -> anyhow::Result<()> {
    if !pem.exists() {
        bail!("{} does not exist yet; start the daemon once to create the CA", pem.display());
    }
    let status = Command::new(SECURITY)
        .args(["add-trusted-cert", "-r", "trustRoot", "-p", "ssl", "-k"])
        .arg(login_keychain())
        .arg(pem)
        .status()
        .context("cannot run security")?;
    if !status.success() {
        bail!("macOS did not add the trust setting (security exited with {status})");
    }
    Ok(())
}

/// Remove the trust setting and the certificate. Missing entries are not an error.
pub fn untrust(pem: &Path, common_name: Option<&str>) -> anyhow::Result<()> {
    if pem.exists() {
        let _ = Command::new(SECURITY).arg("remove-trusted-cert").arg(pem).status();
    }
    if let Some(cn) = common_name.filter(|cn| !cn.is_empty()) {
        // Delete every copy with this exact name (reset makes a new name each time).
        loop {
            let out = Command::new(SECURITY)
                .args(["delete-certificate", "-c", cn])
                .arg(login_keychain())
                .output()
                .context("cannot run security")?;
            if !out.status.success() {
                break;
            }
        }
    }
    Ok(())
}
