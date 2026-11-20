// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Minimal URI handling for ipp, ipps, http, https, dnssd and hp URIs (RFC 3986
//! generic syntax, RFC 3510 for ipp, RFC 7472 for ipps).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uri {
    pub scheme: String,
    /// Host without brackets. Empty for URIs without an authority (hp:/net/...).
    pub host: String,
    pub port: Option<u16>,
    /// Path starting with "/", or empty.
    pub path: String,
    pub query: Option<String>,
}

impl Uri {
    pub fn parse(s: &str) -> Option<Self> {
        let (scheme, rest) = s.split_once(':')?;
        if scheme.is_empty()
            || !scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return None;
        }
        let rest = rest.split('#').next().unwrap_or("");
        let (before_query, query) = match rest.split_once('?') {
            Some((a, q)) => (a, Some(q.to_string())),
            None => (rest, None),
        };
        let (host, port, path) = if let Some(auth_path) = before_query.strip_prefix("//") {
            let (auth, path) = match auth_path.find('/') {
                Some(i) => (&auth_path[..i], &auth_path[i..]),
                None => (auth_path, ""),
            };
            let auth = auth.rsplit('@').next().unwrap_or(auth);
            let (host, port) = split_host_port(auth)?;
            (host, port, path.to_string())
        } else {
            (String::new(), None, before_query.to_string())
        };
        Some(Self {
            scheme: scheme.to_ascii_lowercase(),
            host,
            port,
            path,
            query,
        })
    }

    /// Port, or the scheme default: 631 for ipp/ipps (RFC 3510, RFC 7472),
    /// 80 for http, 443 for https.
    pub fn port_or_default(&self) -> u16 {
        self.port.unwrap_or(match self.scheme.as_str() {
            "ipp" | "ipps" => 631,
            "https" => 443,
            _ => 80,
        })
    }

    pub fn is_tls(&self) -> bool {
        matches!(self.scheme.as_str(), "ipps" | "https")
    }

    /// Path for the HTTP request line, "/" when empty.
    pub fn request_path(&self) -> String {
        let p = if self.path.is_empty() {
            "/"
        } else {
            &self.path
        };
        match &self.query {
            Some(q) => format!("{p}?{q}"),
            None => p.to_string(),
        }
    }

    pub fn query_param(&self, key: &str) -> Option<String> {
        self.query.as_deref()?.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k == key).then(|| percent_decode(v))
        })
    }

    /// Host formatted for a URI or Host header (IPv6 in brackets).
    pub fn host_for_uri(&self) -> String {
        host_for_uri(&self.host)
    }
}

pub fn host_for_uri(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn split_host_port(auth: &str) -> Option<(String, Option<u16>)> {
    if let Some(rest) = auth.strip_prefix('[') {
        let (h, after) = rest.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(p) if !p.is_empty() => Some(p.parse().ok()?),
            _ => None,
        };
        return Some((h.to_string(), port));
    }
    match auth.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() => Some((h.to_string(), Some(p.parse().ok()?))),
        Some((h, _)) => Some((h.to_string(), None)),
        None => Some((auth.to_string(), None)),
    }
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipp_uri() {
        let u = Uri::parse("ipp://192.168.1.147/ipp/print").unwrap();
        assert_eq!(u.host, "192.168.1.147");
        assert_eq!(u.port_or_default(), 631);
        assert_eq!(u.request_path(), "/ipp/print");
    }

    #[test]
    fn ipv6_and_port() {
        let u = Uri::parse("ipps://[fe80::1%25wlan0]:8443/ipp/print").unwrap();
        assert_eq!(u.host, "fe80::1%25wlan0");
        assert_eq!(u.port, Some(8443));
        assert!(u.is_tls());
    }

    #[test]
    fn hp_uri_query() {
        let u = Uri::parse("hp:/net/ENVY_5530_series?ip=192.168.1.147").unwrap();
        assert_eq!(u.host, "");
        assert_eq!(u.query_param("ip").unwrap(), "192.168.1.147");
    }

    #[test]
    fn dnssd_uuid() {
        let u = Uri::parse("dnssd://HP%20ENVY._ipp._tcp.local/?uuid=1c852a4d-b800").unwrap();
        assert_eq!(u.query_param("uuid").unwrap(), "1c852a4d-b800");
        assert_eq!(percent_decode("HP%20ENVY"), "HP ENVY");
    }

    #[test]
    fn bad_input() {
        assert!(Uri::parse("no scheme").is_none());
        assert!(Uri::parse("ipp://host:notaport/").is_none());
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%4"), "%4");
    }
}
