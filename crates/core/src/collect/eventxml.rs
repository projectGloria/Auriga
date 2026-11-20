//! Parse the XML rendering of a Windows event (`EvtRender(EvtRenderEventXml)`).
//! Pure and platform independent so it can be tested with captured XML.
//!
//! Only `System` basics plus `EventData`/`UserData` leaf values are kept.
//! The XML is locale independent; the formatted message is not, which is why
//! the rule engine never looks at the message.

use crate::snapshot::{EventField, EventRecord};
use quick_xml::events::{BytesRef, Event};
use quick_xml::{Reader, XmlVersion};

/// Cap per data value; some providers log multi-KB blobs.
const MAX_VALUE_LEN: usize = 2048;

pub fn parse_event_xml(xml: &str) -> Option<EventRecord> {
    let mut r = Reader::from_str(xml);
    let mut ev = EventRecord::default();
    // Element path below <Event>, by local name.
    let mut path: Vec<String> = Vec::new();
    // Current <Data Name=..> (or UserData leaf) being collected.
    let mut cur: Option<(Option<String>, String)> = None;
    let mut have_id = false;

    loop {
        match r.read_event().ok()? {
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_string();
                let attr = |a: &str| {
                    e.try_get_attribute(a).ok().flatten().and_then(|v| {
                        v.normalized_value(XmlVersion::Implicit1_0)
                            .ok()
                            .map(|c| c.into_owned())
                    })
                };
                match (section(&path), name.as_str()) {
                    (Some(Sec::EventData), "Data") => cur = Some((attr("Name"), String::new())),
                    (Some(Sec::UserData), _) if path.len() >= 3 => {
                        // <UserData><Wrapper><Field>value</Field>...
                        cur = Some((Some(name.clone()), String::new()));
                    }
                    _ => {}
                }
                path.push(name);
            }
            Event::Empty(e) => {
                let name = e.local_name();
                let attr = |a: &str| {
                    e.try_get_attribute(a).ok().flatten().and_then(|v| {
                        v.normalized_value(XmlVersion::Implicit1_0)
                            .ok()
                            .map(|c| c.into_owned())
                    })
                };
                match (section(&path), name.as_ref()) {
                    (Some(Sec::System), "Provider") => {
                        ev.provider = attr("Name").unwrap_or_default()
                    }
                    (Some(Sec::System), "TimeCreated") => {
                        ev.time = attr("SystemTime").map(normalize_time).unwrap_or_default()
                    }
                    (Some(Sec::EventData), "Data") => ev.data.push(EventField {
                        name: attr("Name"),
                        value: String::new(),
                    }),
                    _ => {}
                }
            }
            Event::Text(t) => {
                let text = t.xml10_content();
                push_text(&path, &mut cur, &mut ev, &mut have_id, &text);
            }
            Event::CData(t) => {
                let text = t.to_string();
                push_text(&path, &mut cur, &mut ev, &mut have_id, &text);
            }
            Event::GeneralRef(g) => {
                if let Some(s) = resolve_ref(&g) {
                    push_text(&path, &mut cur, &mut ev, &mut have_id, &s);
                }
            }
            Event::End(_) => {
                let leaving = path.pop();
                if let Some((name, value)) = cur.take() {
                    let is_leaf_end = match section(&path) {
                        Some(Sec::EventData) => leaving.as_deref() == Some("Data"),
                        Some(Sec::UserData) => leaving.as_deref() == name.as_deref(),
                        _ => false,
                    };
                    if is_leaf_end {
                        let mut value = value.trim().to_string();
                        if value.len() > MAX_VALUE_LEN {
                            let mut cut = MAX_VALUE_LEN;
                            while !value.is_char_boundary(cut) {
                                cut -= 1;
                            }
                            value.truncate(cut);
                            value.push('…');
                        }
                        ev.data.push(EventField { name, value });
                    } else {
                        cur = Some((name, value));
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    (have_id && !ev.provider.is_empty()).then_some(ev)
}

#[derive(PartialEq)]
enum Sec {
    System,
    EventData,
    UserData,
}

/// Which top-level section of <Event> the path is inside.
fn section(path: &[String]) -> Option<Sec> {
    match path.get(1).map(String::as_str) {
        Some("System") => Some(Sec::System),
        Some("EventData") => Some(Sec::EventData),
        Some("UserData") => Some(Sec::UserData),
        _ => None,
    }
}

fn push_text(
    path: &[String],
    cur: &mut Option<(Option<String>, String)>,
    ev: &mut EventRecord,
    have_id: &mut bool,
    text: &str,
) {
    if let Some((_, v)) = cur.as_mut() {
        v.push_str(text);
        return;
    }
    if section(path) == Some(Sec::System) && path.len() == 3 {
        let v = text.trim();
        match path[2].as_str() {
            "EventID" => {
                if let Ok(id) = v.parse() {
                    ev.event_id = id;
                    *have_id = true;
                }
            }
            "Level" => ev.level = v.parse().unwrap_or(0),
            "EventRecordID" => ev.record_id = v.parse().unwrap_or(0),
            "Channel" => ev.channel = v.to_string(),
            _ => {}
        }
    }
}

fn resolve_ref(g: &BytesRef<'_>) -> Option<String> {
    if let Ok(Some(c)) = g.resolve_char_ref() {
        return Some(c.to_string());
    }
    Some(
        match g.xml10_content().as_ref() {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" => "'",
            _ => return None,
        }
        .to_string(),
    )
}

/// `2026-11-20T08:22:55.5878831Z` -> `2026-11-20T08:22:55.587Z` (UTC, ms).
/// Unknown formats pass through.
fn normalize_time(s: String) -> String {
    chrono::DateTime::parse_from_rfc3339(&s)
        .map(|d| {
            d.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        })
        .unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: &str = "xmlns='http://schemas.microsoft.com/win/2004/08/events/event'";

    #[test]
    fn named_event_data_with_entities() {
        let xml = format!(
            "<Event {NS}><System><Provider Name='Microsoft-Windows-Kernel-PnP' Guid='{{9c205a39}}'/>\
             <EventID>411</EventID><Version>0</Version><Level>2</Level>\
             <TimeCreated SystemTime='2026-11-19T12:33:54.8411686Z'/><EventRecordID>366</EventRecordID>\
             <Channel>Microsoft-Windows-Kernel-PnP/Configuration</Channel><Security UserID='S-1-5-18'/></System>\
             <EventData><Data Name='DeviceInstanceId'>ACPI\\PNP0C50\\4&amp;6d4ac33&amp;0</Data>\
             <Data Name='LowerFilters'></Data><Data Name='Problem'>0x33</Data></EventData></Event>"
        );
        let e = parse_event_xml(&xml).unwrap();
        assert_eq!(e.provider, "Microsoft-Windows-Kernel-PnP");
        assert_eq!(e.event_id, 411);
        assert_eq!(e.level, 2);
        assert_eq!(e.record_id, 366);
        assert_eq!(e.channel, "Microsoft-Windows-Kernel-PnP/Configuration");
        assert_eq!(e.time, "2026-11-19T12:33:54.841Z");
        assert_eq!(
            e.field("deviceinstanceid"),
            Some("ACPI\\PNP0C50\\4&6d4ac33&0")
        );
        assert_eq!(e.field("LowerFilters"), Some(""));
        assert_eq!(e.field("Problem"), Some("0x33"));
    }

    #[test]
    fn classic_unnamed_data_and_qualifiers() {
        let xml = format!(
            "<Event {NS}><System><Provider Name='Netwtw10'/><EventID Qualifiers='32768'>6062</EventID>\
             <Level>3</Level><TimeCreated SystemTime='2026-11-20T08:21:59.2007601Z'/>\
             <EventRecordID>2499</EventRecordID><Channel>System</Channel><Security/></System>\
             <EventData><Data>\\Device\\NDMP24</Data><Data>Intel(R) Wi-Fi 6 AX201 160MHz</Data>\
             <Binary>0000</Binary></EventData></Event>"
        );
        let e = parse_event_xml(&xml).unwrap();
        assert_eq!(
            (e.provider.as_str(), e.event_id, e.level),
            ("Netwtw10", 6062, 3)
        );
        assert_eq!(e.data.len(), 2);
        assert!(e.data[0].name.is_none());
        assert_eq!(e.data[1].value, "Intel(R) Wi-Fi 6 AX201 160MHz");
    }

    #[test]
    fn user_data_leaves() {
        let xml = format!(
            "<Event {NS}><System><Provider Name='P'/><EventID>7</EventID><Level>4</Level>\
             <TimeCreated SystemTime='2026-01-01T00:00:00Z'/><Channel>C</Channel></System>\
             <UserData><Wrapper xmlns='x'><DeviceInstanceID>USB\\VID_1</DeviceInstanceID>\
             <Status>0</Status></Wrapper></UserData></Event>"
        );
        let e = parse_event_xml(&xml).unwrap();
        assert_eq!(e.field("DeviceInstanceID"), Some("USB\\VID_1"));
        assert_eq!(e.field("Status"), Some("0"));
    }

    #[test]
    fn garbage_is_none() {
        assert!(parse_event_xml("<Event><System></System></Event>").is_none());
        assert!(parse_event_xml("not xml <<<").is_none());
    }
}
