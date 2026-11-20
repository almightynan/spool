// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Devices as the UI sees them, merged from DNS-SD, manual additions and CUPS.

use crate::cups::CupsError;
use crate::monitor::StatusSource;
use spool_protocols::cups::Queue;
use spool_protocols::dnssd::{PrinterService, ScannerService};
use spool_protocols::uri::Uri;

/// Stable row identity. Queues are keyed by name; network devices without a
/// queue by DNS-SD UUID or host.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DeviceId {
    Queue(String),
    Network(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Found with DNS-SD.
    Discovered,
    /// Added by address in this app.
    Manual,
    /// Only known as a CUPS queue.
    QueueOnly,
}

/// A network device: printer and scanner services that belong together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetDevice {
    /// UUID when known, else the host.
    pub key: String,
    pub name: String,
    pub host: String,
    pub make_and_model: Option<String>,
    pub location: Option<String>,
    pub admin_url: Option<String>,
    pub printer: Option<PrinterService>,
    pub scanner: Option<ScannerService>,
    pub origin: Origin,
    /// Manual devices that did not answer the last probe.
    pub unreachable: bool,
    /// The address the user typed, when this device was added by address
    /// (also set when DNS-SD found the same device).
    pub saved_address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub make_and_model: Option<String>,
    pub location: Option<String>,
    pub host: Option<String>,
    pub origin: Origin,
    /// IPP URI of the printer itself, when known.
    pub printer_uri: Option<String>,
    pub scanner: Option<ScannerService>,
    pub admin_url: Option<String>,
    pub queue: Option<Queue>,
    pub is_default: bool,
    pub unreachable: bool,
    pub saved_address: Option<String>,
}

impl Device {
    /// Dashboard source: the printer itself when it speaks IPP, else the queue.
    pub fn status_source(&self) -> Option<StatusSource> {
        if let Some(uri) = &self.printer_uri {
            return Some(StatusSource::Device { uri: uri.clone() });
        }
        self.queue.as_ref().map(|q| StatusSource::Queue {
            name: q.name.clone(),
        })
    }

    pub fn is_printer(&self) -> bool {
        self.printer_uri.is_some() || self.queue.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CupsState {
    Loading,
    Ready,
    Unavailable(CupsError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryState {
    Running,
    /// mDNS could not start (no multicast socket).
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceList {
    pub devices: Vec<Device>,
    pub cups: CupsState,
    pub discovery: DiscoveryState,
}

impl DeviceList {
    pub fn get(&self, id: &DeviceId) -> Option<&Device> {
        self.devices.iter().find(|d| &d.id == id)
    }
}

/// Inputs to [`merge`].
#[derive(Debug, Clone, Default)]
pub struct Sources<'a> {
    pub network: Vec<&'a NetDevice>,
    pub queues: &'a [Queue],
    pub default_queue: Option<&'a str>,
}

fn ipp_device_uri(q: &Queue) -> Option<String> {
    let uri = q.device_uri.as_deref()?;
    let u = Uri::parse(uri)?;
    matches!(u.scheme.as_str(), "ipp" | "ipps").then(|| uri.to_string())
}

fn matches_queue(n: &NetDevice, q: &Queue) -> bool {
    if let (Some(uuid), Some(qu)) = (
        n.printer.as_ref().and_then(|p| p.uuid.as_deref()),
        q.device_uuid(),
    ) {
        if uuid == qu {
            return true;
        }
    }
    q.device_host().is_some_and(|h| {
        h.eq_ignore_ascii_case(&n.host) || n.scanner.as_ref().is_some_and(|s| s.host == h)
    })
}

/// Builds the device list: one row per CUPS queue (merged with the network
/// device it prints to), then one row per network device without a queue.
pub fn merge(src: &Sources<'_>) -> Vec<Device> {
    let mut out = Vec::new();
    let mut used = vec![false; src.network.len()];
    for q in src.queues.iter().filter(|q| !q.is_class) {
        let net = src.network.iter().position(|n| matches_queue(n, q));
        if let Some(i) = net {
            used[i] = true;
        }
        let n = net.map(|i| src.network[i]);
        out.push(Device {
            id: DeviceId::Queue(q.name.clone()),
            name: q.display_name().to_string(),
            make_and_model: n
                .and_then(|n| n.make_and_model.clone())
                .or_else(|| q.make_and_model.clone()),
            location: q
                .location
                .clone()
                .or_else(|| n.and_then(|n| n.location.clone())),
            host: q.device_host().or_else(|| n.map(|n| n.host.clone())),
            origin: n.map(|n| n.origin.clone()).unwrap_or(Origin::QueueOnly),
            printer_uri: n
                .and_then(|n| n.printer.as_ref().map(|p| p.uri.clone()))
                .or_else(|| ipp_device_uri(q)),
            scanner: n.and_then(|n| n.scanner.clone()),
            admin_url: n.and_then(|n| n.admin_url.clone()),
            is_default: src.default_queue == Some(q.name.as_str()),
            queue: Some(q.clone()),
            unreachable: n.is_some_and(|n| n.unreachable),
            saved_address: n.and_then(|n| n.saved_address.clone()),
        });
    }
    for (i, n) in src.network.iter().enumerate() {
        if used[i] {
            continue;
        }
        out.push(Device {
            id: DeviceId::Network(n.key.clone()),
            name: n.name.clone(),
            make_and_model: n.make_and_model.clone(),
            location: n.location.clone(),
            host: Some(n.host.clone()),
            origin: n.origin.clone(),
            printer_uri: n.printer.as_ref().map(|p| p.uri.clone()),
            scanner: n.scanner.clone(),
            admin_url: n.admin_url.clone(),
            queue: None,
            is_default: false,
            unreachable: n.unreachable,
            saved_address: n.saved_address.clone(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use spool_protocols::printer::PrinterState;

    fn queue(name: &str, device_uri: &str) -> Queue {
        Queue {
            name: name.into(),
            uri: format!("ipp://localhost/printers/{name}"),
            device_uri: Some(device_uri.into()),
            info: None,
            location: None,
            make_and_model: Some("Printer - IPP Everywhere".into()),
            state: PrinterState::Idle,
            reasons: vec![],
            accepting_jobs: true,
            is_class: false,
        }
    }

    fn net(host: &str, with_scanner: bool) -> NetDevice {
        NetDevice {
            key: format!("uuid-{host}"),
            name: "HP ENVY 5530 series".into(),
            host: host.into(),
            make_and_model: Some("HP ENVY 5530 series".into()),
            location: None,
            admin_url: Some(format!("http://{host}/")),
            printer: Some(PrinterService {
                instance: "HP ENVY".into(),
                uri: format!("ipp://{host}:631/ipp/print"),
                host: host.into(),
                uuid: Some("1c85".into()),
                make_and_model: None,
                admin_url: None,
                location: None,
                tls: false,
                color: None,
            }),
            scanner: with_scanner.then(|| ScannerService {
                instance: "HP ENVY".into(),
                base_url: format!("http://{host}:80/eSCL/"),
                host: host.into(),
                uuid: None,
                make_and_model: None,
                admin_url: None,
                location: None,
                tls: false,
                inputs: vec![],
                duplex: None,
            }),
            origin: Origin::Manual,
            unreachable: false,
            saved_address: Some(host.into()),
        }
    }

    #[test]
    fn queue_merges_with_its_network_device() {
        let queues = vec![queue("HP_ENVY_5530", "ipp://192.168.1.147/ipp/print")];
        let n = net("192.168.1.147", true);
        let list = merge(&Sources {
            network: vec![&n],
            queues: &queues,
            default_queue: Some("HP_ENVY_5530"),
        });
        assert_eq!(list.len(), 1);
        let d = &list[0];
        assert_eq!(d.id, DeviceId::Queue("HP_ENVY_5530".into()));
        assert!(d.is_default);
        assert!(d.scanner.is_some());
        assert_eq!(d.make_and_model.as_deref(), Some("HP ENVY 5530 series"));
        assert_eq!(
            d.status_source(),
            Some(StatusSource::Device {
                uri: "ipp://192.168.1.147:631/ipp/print".into()
            })
        );
    }

    #[test]
    fn unmatched_rows_and_status_sources() {
        let queues = vec![
            queue("HP_ENVY_5530", "ipp://192.168.1.147/ipp/print"),
            queue("Browsed", "implicitclass://Browsed/"),
        ];
        let other = net("10.0.0.9", false);
        let list = merge(&Sources {
            network: vec![&other],
            queues: &queues,
            default_queue: None,
        });
        assert_eq!(list.len(), 3);
        // No network device: status still comes straight from the printer.
        assert_eq!(
            list[0].status_source(),
            Some(StatusSource::Device {
                uri: "ipp://192.168.1.147/ipp/print".into()
            })
        );
        assert_eq!(
            list[1].status_source(),
            Some(StatusSource::Queue {
                name: "Browsed".into()
            })
        );
        assert_eq!(list[2].id, DeviceId::Network("uuid-10.0.0.9".into()));
        assert!(list[2].queue.is_none());
    }

    #[test]
    fn dnssd_uuid_matches_queue() {
        let queues = vec![queue("Q", "dnssd://HP._ipp._tcp.local/?uuid=1c85")];
        let n = net("192.168.1.147", false);
        let list = merge(&Sources {
            network: vec![&n],
            queues: &queues,
            default_queue: None,
        });
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].host.as_deref(), Some("192.168.1.147"));
    }
}
