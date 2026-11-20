// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! The local user name, needed for requesting-user-name and CUPS PeerCred
//! authentication (cupsd compares it with the socket peer's uid).

use std::os::unix::fs::MetadataExt;

/// Looks up the name for this process's uid in /etc/passwd, falling back to
/// $USER and $LOGNAME. Runs on the worker runtime (file IO).
pub async fn current_user() -> Option<String> {
    let uid = tokio::fs::metadata("/proc/self").await.ok()?.uid();
    if let Ok(passwd) = tokio::fs::read_to_string("/etc/passwd").await {
        if let Some(name) = user_for_uid(&passwd, uid) {
            return Some(name);
        }
    }
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()
        .filter(|u| !u.is_empty())
}

fn user_for_uid(passwd: &str, uid: u32) -> Option<String> {
    passwd.lines().find_map(|line| {
        let mut f = line.split(':');
        let name = f.next()?;
        let _pw = f.next()?;
        let id: u32 = f.next()?.parse().ok()?;
        (id == uid && !name.is_empty()).then(|| name.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwd_lookup() {
        let p = "root:x:0:0:root:/root:/bin/sh\nalice:x:1000:1000::/home/alice:/bin/bash\n";
        assert_eq!(user_for_uid(p, 1000).as_deref(), Some("alice"));
        assert_eq!(user_for_uid(p, 5), None);
    }

    #[tokio::test]
    async fn finds_some_user() {
        assert!(current_user().await.is_some());
    }
}
