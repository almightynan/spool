// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! DNS-SD browsing with the mdns-sd daemon (pure Rust, no Avahi).

use mdns_sd::{ResolvedService, ServiceDaemon, ServiceEvent};
use spool_protocols::dnssd::{ServiceKind, ServiceRecord};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnssdEvent {
    Resolved {
        fullname: String,
        record: ServiceRecord,
    },
    Removed {
        fullname: String,
    },
}

/// Stops browsing when dropped.
pub struct Browser {
    daemon: ServiceDaemon,
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

fn record(kind: ServiceKind, r: &ResolvedService) -> ServiceRecord {
    ServiceRecord {
        kind,
        instance: mdns_sd::instance_name_unescaped(&r.fullname),
        hostname: r.host.clone(),
        port: r.port,
        addresses: r.addresses.iter().map(|a| a.to_ip_addr()).collect(),
        txt: r
            .txt_properties
            .iter()
            .map(|p| (p.key().to_string(), p.val_str().to_string()))
            .collect(),
    }
}

/// Browses the four printer and scanner service types. One forwarding task per
/// type; they end when the daemon shuts down or the receiver is dropped.
pub fn start(rt: &Handle, tx: mpsc::Sender<DnssdEvent>) -> Result<Browser, String> {
    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    for kind in ServiceKind::ALL {
        let rx = daemon
            .browse(kind.service_type())
            .map_err(|e| e.to_string())?;
        let tx = tx.clone();
        rt.spawn(async move {
            while let Ok(ev) = rx.recv_async().await {
                let out = match ev {
                    ServiceEvent::ServiceResolved(r) => DnssdEvent::Resolved {
                        fullname: r.fullname.clone(),
                        record: record(kind, &r),
                    },
                    ServiceEvent::ServiceRemoved(_, fullname) => DnssdEvent::Removed { fullname },
                    _ => continue,
                };
                if tx.send(out).await.is_err() {
                    return;
                }
            }
        });
    }
    Ok(Browser { daemon })
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    #[ignore = "listens on the real network for 8 seconds"]
    async fn browse_real_network() {
        let (tx, mut rx) = mpsc::channel(64);
        let _b = start(&Handle::current(), tx).unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(8), async {
            while let Some(ev) = rx.recv().await {
                if let DnssdEvent::Resolved { record, .. } = &ev {
                    // Fixture format: first line "kind instance host port", then key=value.
                    let mut out = format!(
                        "{:?}\t{}\t{}\t{}\n",
                        record.kind, record.instance, record.hostname, record.port
                    );
                    for (k, v) in &record.txt {
                        out.push_str(&format!("{k}={v}\n"));
                    }
                    if let Ok(dir) = std::env::var("SPOOL_RECORD_TXT") {
                        let name = format!(
                            "{dir}/dnssd-{}-txt.txt",
                            format!("{:?}", record.kind).to_lowercase()
                        );
                        std::fs::write(name, &out).unwrap();
                    }
                    eprintln!("{out}");
                }
            }
        })
        .await;
    }
}
