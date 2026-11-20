//! Typed protocol between the app (pipe server) and the elevated helper
//! (pipe client). Only these enums cross the boundary — there is no
//! "run this string" operation, and unknown fields/variants are rejected.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const PROTOCOL_VERSION: u32 = 2;

/// Hard cap on a single frame; anything larger is a protocol violation.
pub const MAX_FRAME: usize = 1 << 20;

/// Helper exits after this long without a request.
pub const IDLE_TIMEOUT_SECS: u64 = 120;

/// Prefix of every pipe name; the suffix is 32 random hex chars.
pub const PIPE_PREFIX: &str = r"\\.\pipe\auriga-helper-";

/// Messages sent helper -> app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum HelperMessage {
    /// First message after connecting.
    #[serde(rename_all = "camelCase")]
    Hello {
        protocol: u32,
        token: String,
        helper_pid: u32,
        elevated: bool,
    },
    Response {
        id: u64,
        result: HelperResult,
    },
}

/// Messages sent app -> helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum AppMessage {
    Request { id: u64, op: HelperOp },
}

/// The allow-list of operations. Each payload is typed and checked by
/// [`HelperOp::validate`] on both sides; the helper then re-checks every
/// target against the live system before writing anything. No payload holds
/// a path, registry key or command string: the helper derives the one key it
/// may touch from the device itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum HelperOp {
    Ping {
        nonce: u64,
    },
    Shutdown,
    /// Set (`Some`) or delete (`None`, = driver default) one Advanced-tab
    /// value of a network adapter. Returns the previous stored value.
    #[serde(rename_all = "camelCase")]
    SetAdapterProperty {
        instance_id: String,
        keyword: String,
        value: Option<String>,
    },
    /// Write (`Some`) or delete (`None`) a network adapter's
    /// `PnPCapabilities`. Only the "allow the computer to turn off this
    /// device" bit may differ from the current value.
    #[serde(rename_all = "camelCase")]
    SetPnpCapabilities {
        instance_id: String,
        value: Option<u32>,
    },
    /// Restart the device so the driver re-reads its settings.
    #[serde(rename_all = "camelCase")]
    RestartDevice {
        instance_id: String,
    },
    CreateRestorePoint {
        description: String,
    },
    /// Roll back to the driver Windows kept as the previous one. Never
    /// installs anything else.
    #[serde(rename_all = "camelCase")]
    RollbackDriver {
        instance_id: String,
    },
    /// Read the bugcheck header of each file in %SystemRoot%\Minidump.
    /// Read-only; the folder is fixed by the helper, not the request.
    ListMinidumps,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum HelperResult {
    Pong {
        nonce: u64,
    },
    Ok,
    Error {
        message: String,
    },
    PropertySet {
        previous: Option<String>,
    },
    PnpSet {
        previous: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Restarted {
        reboot_required: bool,
    },
    RestorePoint {
        outcome: RestorePointOutcome,
    },
    #[serde(rename_all = "camelCase")]
    RolledBack {
        reboot_required: bool,
    },
    Minidumps {
        files: Vec<MinidumpInfo>,
        warnings: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub enum RestorePointOutcome {
    /// A new restore point exists (its sequence number appeared).
    Created { sequence: u32 },
    /// The call succeeded but no new point appeared: by default Windows
    /// creates at most one per 24 h and silently skips the rest.
    Skipped,
    /// System Protection is off.
    Disabled,
    /// The call succeeded but the result could not be confirmed.
    Unconfirmed { message: String },
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct MinidumpInfo {
    pub file: String,
    #[ts(type = "number")]
    pub size: u64,
    /// RFC 3339.
    pub modified: Option<String>,
    pub bugcheck_code: Option<u32>,
    /// Hex strings: u64 does not fit a JS number.
    pub parameters: Vec<String>,
    /// Why the header could not be read, if it could not.
    pub error: Option<String>,
}

/// Device instance ID as Windows defines it: at most 200 characters
/// (MAX_DEVICE_ID_LEN), printable ASCII above space, no comma.
pub fn valid_instance_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.bytes().all(|b| (0x21..=0x7e).contains(&b) && b != b',')
}

/// Advanced-property keyword (a registry value name under `Ndi\params`).
pub fn valid_keyword(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'*' | b'_' | b'.' | b'-'))
}

/// A property value or restore-point description: short, printable, no
/// control characters.
pub fn valid_text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}

impl HelperOp {
    /// Shape checks only; the helper still verifies the target live.
    pub fn validate(&self) -> Result<(), String> {
        let id = |i: &str| {
            if valid_instance_id(i) {
                Ok(())
            } else {
                Err("invalid device instance ID".to_string())
            }
        };
        match self {
            HelperOp::Ping { .. } | HelperOp::Shutdown | HelperOp::ListMinidumps => Ok(()),
            HelperOp::SetAdapterProperty {
                instance_id,
                keyword,
                value,
            } => {
                id(instance_id)?;
                if !valid_keyword(keyword) {
                    return Err("invalid property keyword".into());
                }
                match value {
                    Some(v) if !valid_text(v, 64) => Err("invalid property value".into()),
                    _ => Ok(()),
                }
            }
            HelperOp::SetPnpCapabilities { instance_id, .. }
            | HelperOp::RestartDevice { instance_id }
            | HelperOp::RollbackDriver { instance_id } => id(instance_id),
            HelperOp::CreateRestorePoint { description } => {
                if valid_text(description, 64) {
                    Ok(())
                } else {
                    Err("invalid restore point description".into())
                }
            }
        }
    }
}

pub fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn random_hex(bytes: usize) -> Result<String, getrandom::Error> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Constant-time comparison for the session token.
pub fn token_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// Command line given to the helper. Values are validated hex, so no
/// quoting/escaping issues are possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperArgs {
    pub pipe: String,
    pub token: String,
    pub parent_pid: u32,
}

impl HelperArgs {
    pub fn to_command_line(&self) -> String {
        format!(
            "--pipe {} --token {} --parent-pid {}",
            self.pipe, self.token, self.parent_pid
        )
    }

    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let (mut pipe, mut token, mut parent) = (None, None, None);
        let mut it = args.into_iter();
        while let Some(a) = it.next() {
            let v = it.next().ok_or_else(|| format!("missing value for {a}"))?;
            match a.as_str() {
                "--pipe" => pipe = Some(v),
                "--token" => token = Some(v),
                "--parent-pid" => parent = Some(v.parse::<u32>().map_err(|_| "bad parent pid")?),
                _ => return Err(format!("unknown argument {a}")),
            }
        }
        let pipe = pipe.ok_or("missing --pipe")?;
        let token = token.ok_or("missing --token")?;
        let parent_pid = parent.ok_or("missing --parent-pid")?;
        match pipe.strip_prefix(PIPE_PREFIX) {
            Some(suffix) if is_hex(suffix, 32) => {}
            _ => return Err("invalid pipe name".into()),
        }
        if !is_hex(&token, 64) {
            return Err("invalid token".into());
        }
        Ok(Self {
            pipe,
            token,
            parent_pid,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn args_roundtrip() {
        let a = HelperArgs {
            pipe: format!("{PIPE_PREFIX}{}", random_hex(16).unwrap()),
            token: random_hex(32).unwrap(),
            parent_pid: 1234,
        };
        let parsed = HelperArgs::parse(a.to_command_line().split(' ').map(String::from)).unwrap();
        assert_eq!(parsed, a);
    }

    #[test]
    fn args_rejects_bad_input() {
        let tok = "a".repeat(64);
        let good_pipe = format!("{PIPE_PREFIX}{}", "0".repeat(32));
        for bad in [
            s(&[
                "--pipe",
                r"\\.\pipe\other",
                "--token",
                &tok,
                "--parent-pid",
                "1",
            ]),
            s(&[
                "--pipe",
                &good_pipe,
                "--token",
                "short",
                "--parent-pid",
                "1",
            ]),
            s(&["--pipe", &good_pipe, "--token", &tok, "--parent-pid", "x"]),
            s(&["--pipe", &good_pipe, "--token", &tok]),
            s(&[
                "--pipe",
                &good_pipe,
                "--token",
                &tok,
                "--parent-pid",
                "1",
                "--evil",
                "x",
            ]),
            s(&[
                "--pipe",
                &format!("{good_pipe}&calc"),
                "--token",
                &tok,
                "--parent-pid",
                "1",
            ]),
        ] {
            assert!(HelperArgs::parse(bad.clone()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn unknown_ops_rejected() {
        let r: Result<AppMessage, _> =
            serde_json::from_str(r#"{"type":"request","id":1,"op":{"op":"runCommand","cmd":"x"}}"#);
        assert!(r.is_err());
        let r: Result<AppMessage, _> = serde_json::from_str(
            r#"{"type":"request","id":1,"op":{"op":"ping","nonce":5,"extra":1}}"#,
        );
        assert!(r.is_err());
        let ok: AppMessage =
            serde_json::from_str(r#"{"type":"request","id":1,"op":{"op":"ping","nonce":5}}"#)
                .unwrap();
        assert_eq!(
            ok,
            AppMessage::Request {
                id: 1,
                op: HelperOp::Ping { nonce: 5 }
            }
        );
    }

    #[test]
    fn op_validation() {
        let ax = r"PCI\VEN_8086&DEV_43F0&SUBSYS_00748086&REV_00\3&11583659&0&A3";
        let set = |id: &str, kw: &str, v: Option<&str>| HelperOp::SetAdapterProperty {
            instance_id: id.into(),
            keyword: kw.into(),
            value: v.map(String::from),
        };
        assert!(set(ax, "MIMOPowerSaveMode", Some("3")).validate().is_ok());
        assert!(set(ax, "*PMARPOffload", None).validate().is_ok());
        let long_id = "x".repeat(201);
        let long_kw = "k".repeat(65);
        let long_v = "9".repeat(65);
        for bad in [
            set("", "k", Some("1")),
            set("A,B", "k", Some("1")),
            set("has space", "k", Some("1")),
            set(&long_id, "k", Some("1")),
            set(ax, r"..\Services", Some("1")),
            set(ax, "a b", Some("1")),
            set(ax, &long_kw, Some("1")),
            set(ax, "k", Some("")),
            set(ax, "k", Some("1\n2")),
            set(ax, "k", Some(&long_v)),
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        let rp = |d: &str| HelperOp::CreateRestorePoint {
            description: d.into(),
        };
        assert!(rp("Auriga: x").validate().is_ok());
        assert!(rp("\u{7}").validate().is_err());
    }

    #[test]
    fn new_ops_wire_format() {
        let m: AppMessage = serde_json::from_str(
            r#"{"type":"request","id":2,"op":{"op":"setAdapterProperty","instanceId":"X","keyword":"K","value":null}}"#,
        )
        .unwrap();
        assert!(matches!(
            m,
            AppMessage::Request {
                op: HelperOp::SetAdapterProperty { value: None, .. },
                ..
            }
        ));
        // No way to smuggle a path or key name.
        let r: Result<AppMessage, _> = serde_json::from_str(
            r#"{"type":"request","id":2,"op":{"op":"setAdapterProperty","instanceId":"X","keyword":"K","value":"1","key":"SOFTWARE"}}"#,
        );
        assert!(r.is_err());
        let r = HelperResult::RestorePoint {
            outcome: RestorePointOutcome::Created { sequence: 7 },
        };
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<HelperResult>(&j).unwrap(), r);
    }

    #[test]
    fn token_compare() {
        assert!(token_eq("abc", "abc"));
        assert!(!token_eq("abc", "abd"));
        assert!(!token_eq("abc", "abcd"));
    }
}
