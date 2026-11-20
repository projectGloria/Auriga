//! Event Log collection via `EvtQuery` + `EvtRender(EvtRenderEventXml)`.
//!
//! Each query is a structured XPath on provider/event ID/level, so nothing
//! depends on the OS language. Messages are formatted best effort for display
//! only. A missing channel or access-denied becomes a warning and the other
//! queries still run.

use super::util::wide;
use crate::collect::eventxml::parse_event_xml;
use crate::collect::{CollectError, ProgressFn};
use crate::snapshot::{EventRecord, SourceWarning};
use std::collections::{HashMap, HashSet};
use tokio_util::sync::CancellationToken;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS, WIN32_ERROR};
use windows::Win32::System::EventLog::{
    EVT_HANDLE, EvtClose, EvtFormatMessage, EvtFormatMessageEvent, EvtNext,
    EvtOpenPublisherMetadata, EvtQuery, EvtQueryChannelPath, EvtQueryReverseDirection,
    EvtQueryTolerateQueryErrors, EvtRender, EvtRenderEventXml,
};
use windows::core::{HRESULT, PCWSTR};

const MAX_MESSAGE: usize = 1500;

struct Spec {
    channel: &'static str,
    /// XPath predicate inside `System[...]`, without the time window.
    select: &'static str,
    cap: usize,
    keep: fn(&EventRecord) -> bool,
}

fn all(_: &EventRecord) -> bool {
    true
}

/// WER 1001 is logged for every app crash; keep only kernel/bugcheck reports.
fn wer_kernel(e: &EventRecord) -> bool {
    e.data.iter().any(|f| {
        f.value.eq_ignore_ascii_case("LiveKernelEvent")
            || f.value.eq_ignore_ascii_case("BlueScreen")
    })
}

const SPECS: &[Spec] = &[
    Spec {
        channel: "System",
        select: "(Level=1 or Level=2 or Level=3)",
        cap: 3000,
        keep: all,
    },
    Spec {
        channel: "System",
        select: "((Provider[@Name='Microsoft-Windows-Kernel-General'] and (EventID=12 or EventID=13)) \
                 or (Provider[@Name='Microsoft-Windows-UserPnp'] and (EventID=20001 or EventID=20003)) \
                 or (Provider[@Name='Microsoft-Windows-WindowsUpdateClient'] and EventID=19) \
                 or (Provider[@Name='Microsoft-Windows-Power-Troubleshooter'] and EventID=1) \
                 or (Provider[@Name='Microsoft-Windows-Kernel-Power'] and EventID=42))",
        cap: 1000,
        keep: all,
    },
    Spec {
        channel: "Microsoft-Windows-WLAN-AutoConfig/Operational",
        select: "(EventID=8001 or EventID=8002 or EventID=8003 or Level=1 or Level=2 or Level=3)",
        cap: 1500,
        keep: all,
    },
    Spec {
        channel: "Microsoft-Windows-Kernel-PnP/Configuration",
        select: "(EventID=400 or EventID=411 or EventID=420 or EventID=430)",
        cap: 1500,
        keep: all,
    },
    Spec {
        channel: "Application",
        select: "Provider[@Name='Windows Error Reporting'] and EventID=1001",
        cap: 500,
        keep: wer_kernel,
    },
];

/// Owned EVT_HANDLE, closed on drop.
struct H(EVT_HANDLE);
impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = EvtClose(self.0);
            }
        }
    }
}

fn win32(e: &windows::core::Error) -> Option<WIN32_ERROR> {
    WIN32_ERROR::from_error(e)
}

pub fn collect(
    days: u32,
    progress: ProgressFn<'_>,
    cancel: &CancellationToken,
    warnings: &mut Vec<SourceWarning>,
) -> Result<Vec<EventRecord>, CollectError> {
    let window_ms = u64::from(days.max(1)) * 86_400_000;
    let mut out: Vec<EventRecord> = Vec::new();
    let mut seen: HashSet<(String, u64)> = HashSet::new();
    let mut fmt = Formatter::default();
    let mut warned: HashSet<&str> = HashSet::new();

    for (i, spec) in SPECS.iter().enumerate() {
        progress("events", i as u32, SPECS.len() as u32);
        if cancel.is_cancelled() {
            return Err(CollectError::Cancelled);
        }
        match query(spec, window_ms, &mut fmt, cancel) {
            Ok(v) => {
                for e in v {
                    if seen.insert((e.channel.clone(), e.record_id)) {
                        out.push(e);
                    }
                }
            }
            Err(QueryError::Cancelled) => return Err(CollectError::Cancelled),
            Err(QueryError::Win(msg)) => {
                if warned.insert(spec.channel) {
                    warnings.push(SourceWarning {
                        source: "events".into(),
                        message: format!("{}: {msg}", spec.channel),
                        instance_id: None,
                    });
                }
            }
        }
    }
    progress("events", SPECS.len() as u32, SPECS.len() as u32);
    // Newest first across channels (RFC 3339 UTC strings sort chronologically).
    out.sort_by(|a, b| b.time.cmp(&a.time));
    Ok(out)
}

enum QueryError {
    Cancelled,
    Win(String),
}

fn query(
    spec: &Spec,
    window_ms: u64,
    fmt: &mut Formatter,
    cancel: &CancellationToken,
) -> Result<Vec<EventRecord>, QueryError> {
    let xpath = format!(
        "*[System[{} and TimeCreated[timediff(@SystemTime) <= {window_ms}]]]",
        spec.select
    );
    let path = wide(spec.channel);
    let q = wide(&xpath);
    let flags = EvtQueryChannelPath.0 | EvtQueryReverseDirection.0 | EvtQueryTolerateQueryErrors.0;
    let rs = unsafe { EvtQuery(None, PCWSTR(path.as_ptr()), PCWSTR(q.as_ptr()), flags) }
        .map(H)
        .map_err(|e| QueryError::Win(describe(&e)))?;

    let mut out = Vec::new();
    let mut buf: Vec<u16> = vec![0; 8192];
    let mut scanned = 0usize;
    'outer: loop {
        let mut batch = [0isize; 64];
        let mut returned = 0u32;
        if let Err(e) = unsafe { EvtNext(rs.0, &mut batch, 2000, 0, &mut returned) } {
            if win32(&e) == Some(ERROR_NO_MORE_ITEMS) {
                break;
            }
            return Err(QueryError::Win(describe(&e)));
        }
        // Take ownership of every returned handle first so all get closed.
        let handles: Vec<H> = batch[..returned as usize]
            .iter()
            .map(|&h| H(EVT_HANDLE(h)))
            .collect();
        if cancel.is_cancelled() {
            return Err(QueryError::Cancelled);
        }
        for h in &handles {
            scanned += 1;
            if let Some(xml) = render_xml(h.0, &mut buf)
                && let Some(mut ev) = parse_event_xml(&xml)
                && (spec.keep)(&ev)
            {
                ev.message = fmt.message(&ev.provider, h.0);
                out.push(ev);
                if out.len() >= spec.cap {
                    break 'outer;
                }
            }
            // Bound work for filtered specs too.
            if scanned >= spec.cap * 20 {
                break 'outer;
            }
        }
        if returned == 0 {
            break;
        }
    }
    Ok(out)
}

fn render_xml(ev: EVT_HANDLE, buf: &mut Vec<u16>) -> Option<String> {
    for _ in 0..2 {
        let mut used = 0u32;
        let mut count = 0u32;
        let bytes = (buf.len() * 2) as u32;
        let r = unsafe {
            EvtRender(
                None,
                ev,
                EvtRenderEventXml.0,
                bytes,
                Some(buf.as_mut_ptr().cast()),
                &mut used,
                &mut count,
            )
        };
        match r {
            Ok(()) => {
                let n = (used as usize / 2).min(buf.len());
                return Some(super::util::from_wide(&buf[..n]));
            }
            Err(e) if win32(&e) == Some(ERROR_INSUFFICIENT_BUFFER) => {
                buf.resize(used as usize / 2 + 1, 0);
            }
            Err(_) => return None,
        }
    }
    None
}

/// Publisher-metadata cache for `EvtFormatMessage`. Providers whose metadata
/// cannot be opened (uninstalled software, access denied) are remembered as
/// `None` so we try once.
#[derive(Default)]
struct Formatter {
    meta: HashMap<String, Option<H>>,
    buf: Vec<u16>,
}

impl Formatter {
    fn message(&mut self, provider: &str, ev: EVT_HANDLE) -> Option<String> {
        let meta = self
            .meta
            .entry(provider.to_string())
            .or_insert_with(|| {
                let p = wide(provider);
                unsafe { EvtOpenPublisherMetadata(None, PCWSTR(p.as_ptr()), PCWSTR::null(), 0, 0) }
                    .ok()
                    .map(H)
            })
            .as_ref()?
            .0;
        if self.buf.is_empty() {
            self.buf = vec![0; 2048];
        }
        for _ in 0..2 {
            let mut used = 0u32;
            let r = unsafe {
                EvtFormatMessage(
                    Some(meta),
                    Some(ev),
                    0,
                    None,
                    EvtFormatMessageEvent.0,
                    Some(&mut self.buf),
                    &mut used,
                )
            };
            match r {
                Ok(()) => {
                    let s =
                        super::util::from_wide(&self.buf[..(used as usize).min(self.buf.len())]);
                    let s = s.trim();
                    if s.is_empty() {
                        return None;
                    }
                    return Some(clip(s));
                }
                Err(e) if win32(&e) == Some(ERROR_INSUFFICIENT_BUFFER) => {
                    // Messages beyond ~64K chars are not worth showing.
                    if used as usize > 65_536 {
                        return None;
                    }
                    self.buf.resize(used as usize + 1, 0);
                }
                // Missing message resources etc.: display falls back to data.
                Err(_) => return None,
            }
        }
        None
    }
}

fn clip(s: &str) -> String {
    match s.char_indices().nth(MAX_MESSAGE) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

fn describe(e: &windows::core::Error) -> String {
    const CHANNEL_NOT_FOUND: HRESULT = HRESULT(0x8007_3A9F_u32 as i32); // ERROR_EVT_CHANNEL_NOT_FOUND
    if e.code() == CHANNEL_NOT_FOUND {
        "channel not found on this system".into()
    } else if e.code() == windows::Win32::Foundation::E_ACCESSDENIED {
        "access denied".into()
    } else {
        format!("query failed ({:#010x})", e.code().0 as u32)
    }
}
