//! Parser for `%windir%\INF\setupapi.dev.log` (device/driver install log).
//!
//! The log text is always English (it is not localised) and structured:
//!
//! ```text
//! >>>  [Device Install (Hardware initiated) - PCI\VEN_8086&DEV_43F0&...]
//! >>>  Section start 2026/11/19 15:52:08.571
//!      ...
//! !!!  dvi: error line
//! !    dvi: warning line
//! <<<  Section end 2026/11/19 15:52:09.190
//! <<<  [Exit status: SUCCESS]            or FAILURE(0xe0000203)
//! ```
//!
//! Timestamps are local time; the caller supplies the zone so tests are
//! deterministic.

use crate::snapshot::{SetupApiSection, SetupApiStatus};
use chrono::{NaiveDateTime, TimeZone};

const MAX_ERRORS: usize = 5;
const MAX_INFS: usize = 12;
const MAX_LINE: usize = 400;

/// Parse the whole log. Returns sections newest first, at most `limit`.
pub fn parse<Tz: TimeZone>(text: &str, tz: &Tz, limit: usize) -> Vec<SetupApiSection>
where
    Tz::Offset: std::fmt::Display,
{
    let mut out = Vec::new();
    let mut cur: Option<SetupApiSection> = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(">>>  ") {
            if let Some(title) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                if let Some(done) = cur.take() {
                    out.push(done);
                }
                cur = Some(SetupApiSection {
                    instance_id: instance_id_from_title(title),
                    title: title.to_string(),
                    ..Default::default()
                });
            } else if let Some(ts) = rest.strip_prefix("Section start ")
                && let Some(s) = cur.as_mut()
            {
                s.start = local_time(ts, tz);
            }
            continue;
        }
        let Some(s) = cur.as_mut() else { continue };
        if let Some(rest) = line.strip_prefix("<<<  ") {
            if let Some(ts) = rest.strip_prefix("Section end ") {
                s.end = local_time(ts, tz);
            } else if let Some(status) = rest
                .strip_prefix("[Exit status: ")
                .and_then(|r| r.strip_suffix(']'))
            {
                if status.starts_with("SUCCESS") {
                    s.status = SetupApiStatus::Success;
                } else {
                    s.status = SetupApiStatus::Failure;
                    s.status_code = status
                        .split_once('(')
                        .and_then(|(_, c)| c.split(')').next())
                        .map(|c| c.trim().to_ascii_lowercase());
                }
                if let Some(done) = cur.take() {
                    out.push(done);
                }
            }
            continue;
        }
        if line.starts_with("!!!") {
            s.error_count += 1;
            if s.errors.len() < MAX_ERRORS {
                s.errors.push(clip(line.trim_start_matches('!').trim()));
            }
        } else if line.starts_with('!') {
            s.warning_count += 1;
        }
        if s.infs.len() < MAX_INFS {
            for inf in infs_in(line) {
                if s.infs.len() < MAX_INFS && !s.infs.contains(&inf) {
                    s.infs.push(inf);
                }
            }
        }
    }
    if let Some(done) = cur.take() {
        out.push(done);
    }
    out.reverse();
    out.truncate(limit);
    out
}

fn local_time<Tz: TimeZone>(ts: &str, tz: &Tz) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    let naive = NaiveDateTime::parse_from_str(ts.trim(), "%Y/%m/%d %H:%M:%S%.3f").ok()?;
    // `earliest` resolves the ambiguous hour at a DST change deterministically.
    tz.from_local_datetime(&naive).earliest().map(|d| {
        d.with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    })
}

/// `Device Install (...) - PCI\VEN_..\3&...` -> the device instance ID.
/// File paths (`C:\...`) and section titles without a target give `None`.
fn instance_id_from_title(title: &str) -> Option<String> {
    let (kind, target) = title.split_once(" - ")?;
    let k = kind.to_ascii_lowercase();
    let device_section = k.starts_with("device install")
        || k.starts_with("restart device")
        || k.starts_with("delete device")
        || k.starts_with("device uninstall")
        || k.starts_with("configure device");
    let t = target.trim();
    let looks_like_path = t.as_bytes().get(1) == Some(&b':') || t.starts_with("\\\\");
    (device_section && t.contains('\\') && !looks_like_path).then(|| t.to_string())
}

/// INF file names mentioned on a line, lower-cased, without directories.
fn infs_in(line: &str) -> Vec<String> {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
        .filter(|t| t.len() > 4 && t.to_ascii_lowercase().ends_with(".inf"))
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

fn clip(s: &str) -> String {
    if s.len() <= MAX_LINE {
        return s.to_string();
    }
    let mut cut = MAX_LINE;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    const LOG: &str = r"[Device Install Log]
     OS Version = 10.0.26100

[BeginLog]

[Boot Session: 2026/11/19 15:51:35.500]

>>>  [Driver Install (DrvSetupInstallDriver) - C:\ProgramData\Cache\WiFiDriver\Netwtw08.INF]
>>>  Section start 2026/11/19 15:52:08.571
     sto: {Import: C:\ProgramData\Cache\WiFiDriver\Netwtw08.INF} 15:52:08.600
<<<  Section end 2026/11/19 15:52:09.190
<<<  [Exit status: SUCCESS (REBOOT_REQUIRED)]


>>>  [Device Install (Hardware initiated) - PCI\VEN_8086&DEV_43F0&SUBSYS_00748086&REV_11\3&11583659&0&A3]
>>>  Section start 2026/11/19 16:00:00.000
     dvi: Selected driver: oem42.inf
!    dvi: Device not started: Device has problem: 0x0a (CM_PROB_FAILED_START).
!!!  dvi: Failed to install device. Error = 0xe0000203
!!!  ndv: Installing NULL driver!
<<<  Section end 2026/11/19 16:00:01.000
<<<  [Exit status: FAILURE(0xE0000203)]

>>>  [Restart Device - USB\VID_1234&PID_0001\5&1]
>>>  Section start 2026/11/19 16:05:00.000
";

    #[test]
    fn parses_sections_newest_first() {
        let tz = FixedOffset::east_opt(3 * 3600).unwrap();
        let v = parse(LOG, &tz, 100);
        assert_eq!(v.len(), 3);

        // Truncated last section: no exit status.
        assert_eq!(v[0].status, SetupApiStatus::Unknown);
        assert_eq!(
            v[0].instance_id.as_deref(),
            Some("USB\\VID_1234&PID_0001\\5&1")
        );

        let f = &v[1];
        assert_eq!(f.status, SetupApiStatus::Failure);
        assert_eq!(f.status_code.as_deref(), Some("0xe0000203"));
        assert_eq!(f.error_count, 2);
        assert_eq!(f.warning_count, 1);
        assert_eq!(
            f.errors[0],
            "dvi: Failed to install device. Error = 0xe0000203"
        );
        assert_eq!(
            f.instance_id.as_deref(),
            Some("PCI\\VEN_8086&DEV_43F0&SUBSYS_00748086&REV_11\\3&11583659&0&A3")
        );
        assert_eq!(f.infs, vec!["oem42.inf"]);
        // Local 16:00 at +03:00 is 13:00Z.
        assert_eq!(f.start.as_deref(), Some("2026-11-19T13:00:00.000Z"));

        let d = &v[2];
        assert_eq!(d.status, SetupApiStatus::Success);
        assert_eq!(d.instance_id, None, "file path target is not a device");
        assert_eq!(d.infs, vec!["netwtw08.inf"]);
    }

    #[test]
    fn limit_and_empty() {
        let tz = FixedOffset::east_opt(0).unwrap();
        assert_eq!(parse(LOG, &tz, 1).len(), 1);
        assert!(parse("", &tz, 10).is_empty());
        assert!(parse("garbage\n<<<  [Exit status: SUCCESS]", &tz, 10).is_empty());
    }
}
