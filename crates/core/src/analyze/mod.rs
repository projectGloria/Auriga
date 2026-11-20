//! Pure analysis over a `SystemSnapshot`: no I/O, no clock (the snapshot's
//! `collected_at` is "now"), so results are reproducible from a dump.

pub mod match_quality;
pub mod patterns;
pub mod timeline;

use crate::snapshot::{EventRecord, SystemSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ts_rs::TS;

pub use match_quality::{Level, MatchAssessment, MatchKind};
pub use patterns::{EventGroup, Signal, SignalKind};
pub use timeline::{TimelineEntry, TimelineKind};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Analysis {
    /// One per device, same order as `snapshot.devices`.
    pub matches: Vec<MatchAssessment>,
    /// Device instance ID each event is about, aligned with `snapshot.events`.
    pub event_device: Vec<Option<String>>,
    /// Named signal each event belongs to, aligned with `snapshot.events`.
    pub event_signal: Vec<Option<SignalKind>>,
    /// Per (channel, provider, event ID) counts, most frequent first.
    pub groups: Vec<EventGroup>,
    /// Only signals that occurred, most severe first.
    pub signals: Vec<Signal>,
    /// Newest first, repeats collapsed.
    pub timeline: Vec<TimelineEntry>,
}

pub fn analyze(s: &SystemSnapshot) -> Analysis {
    let matches = s.devices.iter().map(match_quality::assess).collect();
    let attr = Attributor::new(s);
    let event_device: Vec<Option<String>> = s.events.iter().map(|e| attr.device_for(e)).collect();
    let event_signal: Vec<Option<SignalKind>> = s.events.iter().map(patterns::classify).collect();
    let now = chrono::DateTime::parse_from_rfc3339(&s.collected_at)
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    let groups = patterns::groups(&s.events, &event_device, now);
    let signals = patterns::signals(s, &event_signal, &event_device, now);
    let timeline = timeline::build(s, &event_device, &event_signal);
    Analysis {
        matches,
        event_device,
        event_signal,
        groups,
        signals,
        timeline,
    }
}

/// Maps an event to the device it concerns using only structured values:
/// 1. any data value equal to a device instance ID,
/// 2. `InterfaceGuid` equal to a network adapter's `NetCfgInstanceId`,
/// 3. the provider (or a data value) equal to the service of exactly one
///    present device (e.g. `Netwtw10` for the Intel Wi-Fi driver).
pub struct Attributor {
    by_id: HashMap<String, String>,
    by_guid: HashMap<String, String>,
    by_service: HashMap<String, String>,
}

impl Attributor {
    pub fn new(s: &SystemSnapshot) -> Self {
        let mut by_id = HashMap::new();
        let mut by_guid = HashMap::new();
        let mut services: HashMap<String, Vec<&str>> = HashMap::new();
        for d in &s.devices {
            by_id.insert(d.instance_id.to_ascii_lowercase(), d.instance_id.clone());
            if let Some(g) = &d.net_cfg_instance_id {
                by_guid.insert(g.to_ascii_lowercase(), d.instance_id.clone());
            }
            if d.present
                && let Some(svc) = &d.service
                && svc.len() >= 4
            {
                services
                    .entry(svc.to_ascii_lowercase())
                    .or_default()
                    .push(&d.instance_id);
            }
        }
        let by_service = services
            .into_iter()
            .filter(|(_, v)| v.len() == 1)
            .map(|(k, v)| (k, v[0].to_string()))
            .collect();
        Self {
            by_id,
            by_guid,
            by_service,
        }
    }

    pub fn device_for(&self, e: &EventRecord) -> Option<String> {
        for f in &e.data {
            if f.value.len() > 3
                && let Some(id) = self.by_id.get(&f.value.to_ascii_lowercase())
            {
                return Some(id.clone());
            }
        }
        if let Some(g) = e.field("InterfaceGuid")
            && let Some(id) = self.by_guid.get(&g.to_ascii_lowercase())
        {
            return Some(id.clone());
        }
        if let Some(id) = self.by_service.get(&e.provider.to_ascii_lowercase()) {
            return Some(id.clone());
        }
        e.data
            .iter()
            .find_map(|f| self.by_service.get(&f.value.to_ascii_lowercase()))
            .cloned()
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use crate::snapshot::{DeviceRecord, EventField, EventRecord};

    pub fn dev(id: &str, service: Option<&str>, guid: Option<&str>) -> DeviceRecord {
        DeviceRecord {
            instance_id: id.into(),
            name: id.into(),
            description: None,
            class: Some("Net".into()),
            class_guid: None,
            manufacturer: None,
            enumerator: None,
            location: None,
            service: service.map(Into::into),
            hardware_ids: vec![],
            compatible_ids: vec![],
            present: true,
            status: Default::default(),
            driver: None,
            install: Default::default(),
            power: None,
            net_cfg_instance_id: guid.map(Into::into),
        }
    }

    pub fn ev(
        channel: &str,
        provider: &str,
        id: u32,
        time: &str,
        data: &[(&str, &str)],
    ) -> EventRecord {
        EventRecord {
            channel: channel.into(),
            provider: provider.into(),
            event_id: id,
            level: 4,
            time: time.into(),
            record_id: 0,
            data: data
                .iter()
                .map(|(n, v)| EventField {
                    name: (!n.is_empty()).then(|| n.to_string()),
                    value: v.to_string(),
                })
                .collect(),
            message: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::*;

    #[test]
    fn attribution_by_id_guid_and_service() {
        let s = SystemSnapshot {
            collected_at: "2026-11-20T12:00:00Z".into(),
            devices: vec![
                dev(
                    "PCI\\VEN_8086&DEV_43F0\\3&1",
                    Some("Netwtw10"),
                    Some("{606CA768-EAC7-4E22-93DA-9ABBCF153B28}"),
                ),
                dev("USB\\A\\1", Some("usbhub"), None),
                dev("USB\\B\\1", Some("usbhub"), None),
            ],
            ..Default::default()
        };
        let a = Attributor::new(&s);
        let t = "2026-11-20T11:00:00Z";
        let by_id = ev(
            "x",
            "Microsoft-Windows-Kernel-PnP",
            411,
            t,
            &[("DeviceInstanceId", "pci\\ven_8086&dev_43f0\\3&1")],
        );
        let by_guid = ev(
            "x",
            "Microsoft-Windows-WLAN-AutoConfig",
            8003,
            t,
            &[("InterfaceGuid", "{606ca768-eac7-4e22-93da-9abbcf153b28}")],
        );
        let by_provider = ev("System", "Netwtw10", 6062, t, &[("", "\\Device\\NDMP24")]);
        let ambiguous = ev("System", "usbhub", 1, t, &[]);
        let id = Some("PCI\\VEN_8086&DEV_43F0\\3&1".to_string());
        assert_eq!(a.device_for(&by_id), id);
        assert_eq!(a.device_for(&by_guid), id);
        assert_eq!(a.device_for(&by_provider), id);
        assert_eq!(
            a.device_for(&ambiguous),
            None,
            "service shared by two devices"
        );
    }
}
