// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! The device registry: one task that merges DNS-SD results, manually added
//! devices and CUPS queues into a [`DeviceList`] published on a watch channel.

use super::mdns::{self, Browser, DnssdEvent};
use super::probe::{probe, ProbeError};
use crate::cups::{CupsClient, CupsError};
use crate::model::{merge, CupsState, DeviceList, DiscoveryState, NetDevice, Origin, Sources};
use crate::net::CancelToken;
use crate::poll::PollControl;
use spool_protocols::cups::Queue;
use spool_protocols::dnssd::{PrinterService, ScannerService, ServiceKind, ServiceRecord};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch, Notify};

enum Cmd {
    AddManual {
        input: String,
        reply: oneshot::Sender<Result<NetDevice, ProbeError>>,
    },
    RemoveManual {
        host: String,
    },
    Reprobe,
}

enum Msg {
    Cups(Result<(Vec<Queue>, Option<String>), CupsError>),
    /// Boxed: a probed device is much larger than the other messages.
    Probed(Box<(String, Result<NetDevice, ProbeError>)>),
}

/// Handle owned by the app. Dropping it stops the registry and discovery.
pub struct Registry {
    cmd: mpsc::Sender<Cmd>,
    pub list: watch::Receiver<DeviceList>,
    cups_refresh: Arc<Notify>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    _browser: Option<Browser>,
}

impl Drop for Registry {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    Stopped,
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the device list is not running")
    }
}

impl Registry {
    /// Probes an address and adds it as a manual device.
    pub async fn add_manual(
        &self,
        input: String,
    ) -> Result<Result<NetDevice, ProbeError>, RegistryError> {
        let (reply, rx) = oneshot::channel();
        self.cmd
            .send(Cmd::AddManual { input, reply })
            .await
            .map_err(|_| RegistryError::Stopped)?;
        rx.await.map_err(|_| RegistryError::Stopped)
    }

    pub async fn remove_manual(&self, host: String) -> Result<(), RegistryError> {
        self.cmd
            .send(Cmd::RemoveManual { host })
            .await
            .map_err(|_| RegistryError::Stopped)
    }

    /// Re-reads CUPS and re-probes manual devices now.
    pub fn refresh(&self) {
        self.cups_refresh.notify_one();
        let _ = self.cmd.try_send(Cmd::Reprobe);
    }
}

struct Manual {
    input: String,
    device: Option<NetDevice>,
    probing: bool,
}

struct State {
    dnssd: BTreeMap<String, ServiceRecord>,
    manual: Vec<Manual>,
    queues: Vec<Queue>,
    default_queue: Option<String>,
    cups: CupsState,
    discovery: DiscoveryState,
}

/// Groups DNS-SD services by device UUID (or host when there is none).
/// Plain ipp and uscan are preferred over their TLS variants (docs/PLAN.md).
pub fn group_services(records: &[&ServiceRecord]) -> Vec<NetDevice> {
    let mut groups: BTreeMap<String, Vec<&ServiceRecord>> = BTreeMap::new();
    for r in records {
        let key = r.uuid().unwrap_or_else(|| r.best_host());
        groups.entry(key).or_default().push(r);
    }
    groups
        .into_iter()
        .map(|(key, rs)| {
            let pick = |a: ServiceKind, b: ServiceKind| {
                rs.iter()
                    .find(|r| r.kind == a)
                    .or_else(|| rs.iter().find(|r| r.kind == b))
                    .copied()
            };
            let p = pick(ServiceKind::Ipp, ServiceKind::Ipps);
            let s = pick(ServiceKind::Uscan, ServiceKind::Uscans);
            let printer = p.and_then(PrinterService::from_record);
            let scanner = s.and_then(ScannerService::from_record);
            let first = p.or(s).unwrap_or(rs[0]);
            NetDevice {
                key,
                name: first.instance.clone(),
                host: first.best_host(),
                make_and_model: first.txt("ty").map(str::to_string),
                location: first.txt("note").map(str::to_string),
                admin_url: p
                    .and_then(|r| r.txt("adminurl"))
                    .or_else(|| s.and_then(|r| r.txt("adminurl")))
                    .map(str::to_string),
                printer,
                scanner,
                origin: Origin::Discovered,
                unreachable: false,
                saved_address: None,
            }
        })
        .collect()
}

impl State {
    fn apply_probe(&mut self, input: &str, device: Option<&NetDevice>) {
        match self.manual.iter_mut().find(|m| m.input == input) {
            Some(m) => {
                m.probing = false;
                match device {
                    Some(d) => m.device = Some(d.clone()),
                    // Keep the last good details, marked as not answering.
                    None => {
                        if let Some(d) = m.device.as_mut() {
                            d.unreachable = true;
                        }
                    }
                }
            }
            None => self.manual.push(Manual {
                input: input.to_string(),
                device: device.cloned(),
                probing: false,
            }),
        }
    }

    fn snapshot(&self) -> DeviceList {
        let records: Vec<&ServiceRecord> = self.dnssd.values().collect();
        let mut discovered = group_services(&records);
        let manual: Vec<NetDevice> = self
            .manual
            .iter()
            .map(|m| match &m.device {
                Some(d) => NetDevice {
                    saved_address: Some(m.input.clone()),
                    ..d.clone()
                },
                None => NetDevice {
                    key: m.input.clone(),
                    name: m.input.clone(),
                    host: m.input.clone(),
                    make_and_model: None,
                    location: None,
                    admin_url: None,
                    printer: None,
                    scanner: None,
                    origin: Origin::Manual,
                    unreachable: !m.probing,
                    saved_address: Some(m.input.clone()),
                },
            })
            .collect();
        // A device found by DNS-SD carries more detail than a manual probe;
        // it keeps the saved address so it can still be forgotten.
        let manual: Vec<NetDevice> = manual
            .into_iter()
            .filter(|m| {
                match discovered
                    .iter_mut()
                    .find(|d| d.host == m.host || d.key == m.key)
                {
                    Some(d) => {
                        d.saved_address = m.saved_address.clone();
                        false
                    }
                    None => true,
                }
            })
            .collect();
        let network: Vec<&NetDevice> = discovered.iter().chain(manual.iter()).collect();
        DeviceList {
            devices: merge(&Sources {
                network,
                queues: &self.queues,
                default_queue: self.default_queue.as_deref(),
            }),
            cups: self.cups.clone(),
            discovery: self.discovery.clone(),
        }
    }
}

pub fn spawn_registry(
    rt: &Handle,
    cups: Arc<CupsClient>,
    poll: &PollControl,
    manual_hosts: Vec<String>,
) -> Registry {
    let (msg_tx, mut msg_rx) = mpsc::channel::<Msg>(64);
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<Cmd>(8);
    let (dnssd_tx, mut dnssd_rx) = mpsc::channel::<DnssdEvent>(64);
    let cups_refresh = Arc::new(Notify::new());
    let mut tasks = Vec::new();

    let (browser, discovery) = match mdns::start(rt, dnssd_tx) {
        Ok(b) => (Some(b), DiscoveryState::Running),
        Err(e) => (None, DiscoveryState::Unavailable(e)),
    };

    let mut state = State {
        dnssd: BTreeMap::new(),
        manual: manual_hosts
            .into_iter()
            .map(|input| Manual {
                input,
                device: None,
                probing: true,
            })
            .collect(),
        queues: Vec::new(),
        default_queue: None,
        cups: CupsState::Loading,
        discovery,
    };
    let (list_tx, list_rx) = watch::channel(state.snapshot());

    // CUPS poller: queues and the default queue, on the shared schedule.
    tasks.push(rt.spawn({
        let cups = cups.clone();
        let refresh = cups_refresh.clone();
        let tx = msg_tx.clone();
        let mut waiter = poll.waiter();
        async move {
            let cancel = CancelToken::new();
            loop {
                let r = match cups.queues(&cancel).await {
                    Ok(q) => cups.default_queue(&cancel).await.map(|d| (q, d)),
                    Err(e) => Err(e),
                };
                if tx.send(Msg::Cups(r)).await.is_err() || !waiter.wait(&refresh).await {
                    return;
                }
            }
        }
    }));

    let probe_all = {
        let cups = cups.clone();
        let tx = msg_tx.clone();
        move |inputs: Vec<String>| {
            let cups = cups.clone();
            let tx = tx.clone();
            async move {
                for input in inputs {
                    let result = probe(&input, cups.user(), &CancelToken::new()).await;
                    if tx
                        .send(Msg::Probed(Box::new((input, result))))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
    };

    // Probe saved manual devices once at start, one after another.
    let initial: Vec<String> = state.manual.iter().map(|m| m.input.clone()).collect();
    if !initial.is_empty() {
        tasks.push(rt.spawn(probe_all(initial)));
    }

    tasks.push(rt.spawn({
        let rt = rt.clone();
        let cups = cups.clone();
        let msg_tx = msg_tx.clone();
        async move {
            // Replies for manual adds wait here until their probe finishes.
            let mut pending: Vec<(String, oneshot::Sender<Result<NetDevice, ProbeError>>)> = Vec::new();
            let mut reprobe: Option<tokio::task::JoinHandle<()>> = None;
            loop {
                tokio::select! {
                    Some(ev) = dnssd_rx.recv() => match ev {
                        DnssdEvent::Resolved { fullname, record } => {
                            state.dnssd.insert(fullname, record);
                        }
                        DnssdEvent::Removed { fullname } => {
                            state.dnssd.remove(&fullname);
                        }
                    },
                    Some(msg) = msg_rx.recv() => match msg {
                        Msg::Cups(Ok((queues, default))) => {
                            state.queues = queues;
                            state.default_queue = default;
                            state.cups = CupsState::Ready;
                        }
                        Msg::Cups(Err(e)) => {
                            state.queues.clear();
                            state.default_queue = None;
                            state.cups = CupsState::Unavailable(e);
                        }
                        Msg::Probed(probed) => {
                            let (input, result) = *probed;
                            let reply = pending
                                .iter()
                                .position(|(p, _)| *p == input)
                                .map(|i| pending.remove(i).1);
                            // A failed first add is reported but not kept.
                            if reply.is_none() || result.is_ok() {
                                state.apply_probe(&input, result.as_ref().ok());
                            }
                            if let Some(reply) = reply {
                                let _ = reply.send(result);
                            }
                        }
                    },
                    Some(cmd) = cmd_rx.recv() => match cmd {
                        Cmd::AddManual { input, reply } => {
                            let input = input.trim().to_string();
                            pending.push((input.clone(), reply));
                            let cups = cups.clone();
                            let tx = msg_tx.clone();
                            rt.spawn(async move {
                                let result = probe(&input, cups.user(), &CancelToken::new()).await;
                                let _ = tx.send(Msg::Probed(Box::new((input, result)))).await;
                            });
                        }
                        Cmd::RemoveManual { host } => {
                            state.manual.retain(|m| m.input != host && m.device.as_ref().map(|d| &d.host) != Some(&host));
                        }
                        Cmd::Reprobe => {
                            // At most one re-probe pass at a time.
                            if reprobe.as_ref().is_none_or(|h| h.is_finished()) {
                                let inputs = state.manual.iter().map(|m| m.input.clone()).collect();
                                reprobe = Some(rt.spawn(probe_all(inputs)));
                            }
                        }
                    },
                    else => return,
                }
                let snap = state.snapshot();
                list_tx.send_if_modified(|cur| {
                    if *cur != snap {
                        *cur = snap;
                        true
                    } else {
                        false
                    }
                });
            }
        }
    }));

    Registry {
        cmd: cmd_tx,
        list: list_rx,
        cups_refresh,
        tasks,
        _browser: browser,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(kind: ServiceKind, uuid: &str, host: &str) -> ServiceRecord {
        ServiceRecord {
            kind,
            instance: "HP ENVY 5530 series [6A3607]".into(),
            hostname: "HP6A3607.local.".into(),
            port: if kind.is_printer() { 631 } else { 80 },
            addresses: vec![host.parse().unwrap()],
            txt: vec![
                ("UUID".into(), uuid.into()),
                ("ty".into(), "HP ENVY 5530 series".into()),
                ("rp".into(), "ipp/print".into()),
                ("adminurl".into(), format!("http://{host}/")),
            ],
        }
    }

    #[test]
    fn printer_and_scanner_with_same_uuid_are_one_device() {
        let a = rec(ServiceKind::Ipp, "1C85", "192.168.1.147");
        let b = rec(ServiceKind::Ipps, "1c85", "192.168.1.147");
        let c = rec(ServiceKind::Uscan, "1c85", "192.168.1.147");
        let d = rec(ServiceKind::Ipp, "ffff", "192.168.1.5");
        let devs = group_services(&[&a, &b, &c, &d]);
        assert_eq!(devs.len(), 2);
        let envy = devs.iter().find(|d| d.key == "1c85").unwrap();
        assert!(
            !envy.printer.as_ref().unwrap().tls,
            "plain ipp is preferred"
        );
        assert_eq!(
            envy.scanner.as_ref().unwrap().base_url,
            "http://192.168.1.147:80/eSCL/"
        );
        assert_eq!(envy.admin_url.as_deref(), Some("http://192.168.1.147/"));
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    #[test]
    fn manual_entry_merged_into_discovered_keeps_saved_address() {
        let rec = ServiceRecord {
            kind: ServiceKind::Ipp,
            instance: "Office".into(),
            hostname: "office.local.".into(),
            port: 631,
            addresses: vec!["10.0.0.5".parse().unwrap()],
            txt: vec![
                ("UUID".into(), "abc".into()),
                ("rp".into(), "ipp/print".into()),
            ],
        };
        let mut dnssd = BTreeMap::new();
        dnssd.insert("Office._ipp._tcp.local.".to_string(), rec);
        let state = State {
            dnssd,
            manual: vec![Manual {
                input: "10.0.0.5".into(),
                device: None,
                probing: true,
            }],
            queues: vec![],
            default_queue: None,
            cups: CupsState::Ready,
            discovery: DiscoveryState::Running,
        };
        let list = state.snapshot();
        assert_eq!(list.devices.len(), 1);
        assert_eq!(list.devices[0].saved_address.as_deref(), Some("10.0.0.5"));
    }
}
