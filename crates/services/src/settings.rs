// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! App settings in `$XDG_CONFIG_HOME/spool/settings.conf` (inside Flatpak,
//! `~/.var/app/<id>/config/...`). One `key=value` per line; unknown keys and
//! bad values fall back to defaults so an edited file never stops the app.

use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Follow the system light/dark preference.
    System,
    Light,
    Dark,
}

impl Style {
    fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

pub const REFRESH_MIN: u32 = 5;
pub const REFRESH_MAX: u32 = 300;
pub const REFRESH_DEFAULT: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub refresh_secs: u32,
    /// None means the user's Pictures folder, resolved by the UI.
    pub save_folder: Option<PathBuf>,
    pub style: Style,
    /// Devices added by address, re-probed at start.
    pub manual_hosts: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            refresh_secs: REFRESH_DEFAULT,
            save_folder: None,
            style: Style::System,
            manual_hosts: Vec::new(),
        }
    }
}

impl Settings {
    pub fn refresh_interval(&self) -> Duration {
        Duration::from_secs(u64::from(self.refresh_secs))
    }

    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "refresh_secs" => {
                    if let Ok(n) = v.parse::<u32>() {
                        s.refresh_secs = n.clamp(REFRESH_MIN, REFRESH_MAX);
                    }
                }
                "save_folder" if !v.is_empty() => s.save_folder = Some(PathBuf::from(v)),
                "style" => {
                    s.style = match v {
                        "light" => Style::Light,
                        "dark" => Style::Dark,
                        _ => Style::System,
                    }
                }
                "manual_host" if !v.is_empty() && !s.manual_hosts.iter().any(|h| h == v) => {
                    s.manual_hosts.push(v.to_string())
                }
                _ => {}
            }
        }
        s
    }

    pub fn serialize(&self) -> String {
        let mut out = String::from("# spool settings\n");
        out.push_str(&format!("refresh_secs={}\n", self.refresh_secs));
        if let Some(f) = &self.save_folder {
            // Paths with newlines cannot be stored in this format; skip them.
            let f = f.to_string_lossy();
            if !f.contains('\n') {
                out.push_str(&format!("save_folder={f}\n"));
            }
        }
        out.push_str(&format!("style={}\n", self.style.key()));
        for h in &self.manual_hosts {
            out.push_str(&format!("manual_host={h}\n"));
        }
        out
    }

    /// Reads settings; a missing file gives the defaults.
    pub async fn load(path: &Path) -> std::io::Result<Self> {
        match tokio::fs::read_to_string(path).await {
            Ok(t) => Ok(Self::parse(&t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Writes to a temporary file and renames it, so a crash never leaves a
    /// half-written file.
    pub async fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let tmp = path.with_extension("conf.tmp");
        tokio::fs::write(&tmp, self.serialize()).await?;
        tokio::fs::rename(&tmp, path).await
    }
}

/// `$XDG_CONFIG_HOME/spool/settings.conf`, or `~/.config/...`.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("spool").join("settings.conf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Settings {
            refresh_secs: 30,
            save_folder: Some("/home/a/Scans".into()),
            style: Style::Dark,
            manual_hosts: vec!["192.168.1.147".into(), "printer.local".into()],
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn bad_values_fall_back() {
        let s =
            Settings::parse("refresh_secs=1\nstyle=neon\nnonsense\nmanual_host=a\nmanual_host=a\n");
        assert_eq!(s.refresh_secs, REFRESH_MIN);
        assert_eq!(s.style, Style::System);
        assert_eq!(s.manual_hosts, vec!["a"]);
        assert_eq!(
            Settings::parse("refresh_secs=x").refresh_secs,
            REFRESH_DEFAULT
        );
    }

    #[tokio::test]
    async fn load_missing_and_save() {
        let dir = std::env::temp_dir().join(format!("spool-settings-{}", std::process::id()));
        let path = dir.join("sub").join("settings.conf");
        assert_eq!(Settings::load(&path).await.unwrap(), Settings::default());
        let s = Settings {
            refresh_secs: 60,
            ..Settings::default()
        };
        s.save(&path).await.unwrap();
        assert_eq!(Settings::load(&path).await.unwrap(), s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
