// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! HPLIP maintenance tools, wrapped as external programs.
//!
//! From hplip 3.22.10 sources (base/module.py, clean.py, align.py, info.py,
//! makeuri.py):
//! - hp-clean and hp-align accept `-d <device-uri>` and run in GUI mode with
//!   `-u`, where HPLIP itself walks the person through the steps. Both only
//!   accept devices installed as a CUPS queue on the HPLIP `hp:` backend
//!   (getDeviceUri with restrict_to_installed_devices=True).
//! - hp-info `-i -x -d <uri>` prints device details as text; `-x` lifts the
//!   installed-queue restriction.
//! - hp-makeuri `-c <ip>` prints only the CUPS (hp:) URI for a network device.
//!
//! Inside the Flatpak these host programs cannot run, so maintenance is off
//! there (decision recorded in docs/PLAN.md).

use crate::net::CancelToken;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const INFO_TIMEOUT: Duration = Duration::from_secs(60);
const MAKEURI_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HplipTools {
    pub clean: Option<PathBuf>,
    pub align: Option<PathBuf>,
    pub info: Option<PathBuf>,
    pub makeuri: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HplipAvailability {
    /// Running inside Flatpak: host tools are out of reach.
    Sandboxed,
    NotInstalled,
    Available(HplipTools),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HplipError {
    Spawn(String),
    Failed {
        code: Option<i32>,
        output: String,
    },
    Timeout,
    Cancelled,
    /// hp-makeuri found no HPLIP device at the address.
    NotHpDevice,
}

impl fmt::Display for HplipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "could not start the HPLIP tool: {e}"),
            Self::Failed { code: Some(c), .. } => {
                write!(f, "the HPLIP tool stopped with exit code {c}")
            }
            Self::Failed { code: None, .. } => write!(f, "the HPLIP tool was stopped"),
            Self::Timeout => write!(f, "the HPLIP tool did not finish in time"),
            Self::Cancelled => write!(f, "canceled"),
            Self::NotHpDevice => write!(f, "HPLIP does not recognize this printer"),
        }
    }
}

impl std::error::Error for HplipError {}

async fn find_in_path(name: &str, path_var: &str) -> Option<PathBuf> {
    for dir in std::env::split_paths(path_var) {
        let candidate = dir.join(name);
        if let Ok(m) = tokio::fs::metadata(&candidate).await {
            use std::os::unix::fs::PermissionsExt;
            if m.is_file() && m.permissions().mode() & 0o111 != 0 {
                return Some(candidate);
            }
        }
    }
    None
}

pub async fn detect() -> HplipAvailability {
    // flatpak(1): every sandbox has /.flatpak-info.
    if tokio::fs::metadata("/.flatpak-info").await.is_ok() {
        return HplipAvailability::Sandboxed;
    }
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
    detect_in(&path).await
}

async fn detect_in(path: &str) -> HplipAvailability {
    let tools = HplipTools {
        clean: find_in_path("hp-clean", path).await,
        align: find_in_path("hp-align", path).await,
        info: find_in_path("hp-info", path).await,
        makeuri: find_in_path("hp-makeuri", path).await,
    };
    if tools.clean.is_none() && tools.align.is_none() && tools.info.is_none() {
        HplipAvailability::NotInstalled
    } else {
        HplipAvailability::Available(tools)
    }
}

/// Removes ANSI escape sequences (HPLIP prints bold text with them).
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

async fn run(
    program: &Path,
    args: &[&str],
    timeout: Option<Duration>,
    cancel: &CancelToken,
) -> Result<String, HplipError> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| HplipError::Spawn(e.to_string()))?;
    let wait = child.wait_with_output();
    let out = tokio::select! {
        _ = cancel.cancelled() => return Err(HplipError::Cancelled),
        r = async {
            match timeout {
                Some(t) => tokio::time::timeout(t, wait).await.map_err(|_| HplipError::Timeout),
                None => Ok(wait.await),
            }
        } => r?.map_err(|e| HplipError::Spawn(e.to_string()))?,
    };
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        text.push('\n');
        text.push_str(&err);
    }
    let mut text = strip_ansi(&text);
    if text.len() > MAX_OUTPUT {
        let mut cut = MAX_OUTPUT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    if out.status.success() {
        Ok(text)
    } else {
        Err(HplipError::Failed {
            code: out.status.code(),
            output: text,
        })
    }
}

/// HPLIP device URI for a network address (`hp-makeuri -c <ip>`).
pub async fn make_uri(
    tools: &HplipTools,
    host: &str,
    cancel: &CancelToken,
) -> Result<String, HplipError> {
    let prog = tools.makeuri.as_deref().ok_or(HplipError::NotHpDevice)?;
    let out = match run(prog, &["-c", host], Some(MAKEURI_TIMEOUT), cancel).await {
        Ok(o) => o,
        Err(HplipError::Failed { .. }) => return Err(HplipError::NotHpDevice),
        Err(e) => return Err(e),
    };
    out.lines()
        .map(str::trim)
        .find(|l| l.starts_with("hp:/"))
        .map(str::to_string)
        .ok_or(HplipError::NotHpDevice)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiTool {
    Clean,
    Align,
}

/// Opens hp-clean or hp-align in HPLIP's own window and waits until it is
/// closed. No timeout: the person decides how long the steps take.
pub async fn run_gui(
    tools: &HplipTools,
    tool: GuiTool,
    device_uri: &str,
    cancel: &CancelToken,
) -> Result<(), HplipError> {
    let prog = match tool {
        GuiTool::Clean => tools.clean.as_deref(),
        GuiTool::Align => tools.align.as_deref(),
    }
    .ok_or_else(|| HplipError::Spawn("tool not installed".into()))?;
    run(prog, &["-u", "-d", device_uri], None, cancel)
        .await
        .map(|_| ())
}

/// Device details from `hp-info -i -x -d <uri>`.
pub async fn diagnostics(
    tools: &HplipTools,
    device_uri: &str,
    cancel: &CancelToken,
) -> Result<String, HplipError> {
    let prog = tools
        .info
        .as_deref()
        .ok_or_else(|| HplipError::Spawn("hp-info not installed".into()))?;
    run(
        prog,
        &["-i", "-x", "-d", device_uri],
        Some(INFO_TIMEOUT),
        cancel,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_is_removed() {
        assert_eq!(strip_ansi("\u{1b}[01mBold\u{1b}[0m text"), "Bold text");
    }

    async fn fake_tools(dir: &Path, script: &str) -> HplipTools {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::create_dir_all(dir).await.unwrap();
        for name in ["hp-clean", "hp-align", "hp-info", "hp-makeuri"] {
            let p = dir.join(name);
            tokio::fs::write(&p, script).await.unwrap();
            tokio::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))
                .await
                .unwrap();
        }
        match detect_in(&dir.to_string_lossy()).await {
            HplipAvailability::Available(t) => t,
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn detection_and_runs() {
        let dir = std::env::temp_dir().join(format!("spool-hplip-{}", std::process::id()));
        assert_eq!(
            detect_in("/nonexistent").await,
            HplipAvailability::NotInstalled
        );
        let t = fake_tools(
            &dir,
            "#!/bin/sh\necho \"args: $*\"\necho 'hp:/net/ENVY_5530_series?ip=10.0.0.5'\n",
        )
        .await;
        let c = CancelToken::new();
        assert_eq!(
            make_uri(&t, "10.0.0.5", &c).await.unwrap(),
            "hp:/net/ENVY_5530_series?ip=10.0.0.5"
        );
        let out = diagnostics(&t, "hp:/net/x?ip=1", &c).await.unwrap();
        assert!(out.contains("args: -i -x -d hp:/net/x?ip=1"));
        run_gui(&t, GuiTool::Clean, "hp:/net/x?ip=1", &c)
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn failures_and_cancel() {
        let dir = std::env::temp_dir().join(format!("spool-hplip-f-{}", std::process::id()));
        let t = fake_tools(&dir, "#!/bin/sh\necho no >&2\nexit 3\n").await;
        let c = CancelToken::new();
        assert_eq!(
            make_uri(&t, "10.0.0.5", &c).await,
            Err(HplipError::NotHpDevice)
        );
        assert!(matches!(
            diagnostics(&t, "hp:/x", &c).await,
            Err(HplipError::Failed { code: Some(3), .. })
        ));
        let slow = fake_tools(&dir, "#!/bin/sh\nsleep 30\n").await;
        let c2 = c.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            c2.cancel();
        });
        assert_eq!(
            run_gui(&slow, GuiTool::Align, "hp:/x", &c).await,
            Err(HplipError::Cancelled)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
