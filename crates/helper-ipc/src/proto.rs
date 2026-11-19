//! Typed protocol between the app (pipe server) and the elevated helper
//! (pipe client). Only these enums cross the boundary — there is no
//! "run this string" operation, and unknown fields/variants are rejected.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

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

/// The allow-list of operations. Phase 1 has only Ping/Shutdown; fix ops
/// (adapter property, restore point, rollback…) are added in Phase 4, each
/// with its own typed, validated payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum HelperOp {
    Ping { nonce: u64 },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum HelperResult {
    Pong { nonce: u64 },
    Ok,
    Error { message: String },
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
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
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
        let parsed =
            HelperArgs::parse(a.to_command_line().split(' ').map(String::from)).unwrap();
        assert_eq!(parsed, a);
    }

    #[test]
    fn args_rejects_bad_input() {
        let tok = "a".repeat(64);
        let good_pipe = format!("{PIPE_PREFIX}{}", "0".repeat(32));
        for bad in [
            s(&["--pipe", r"\\.\pipe\other", "--token", &tok, "--parent-pid", "1"]),
            s(&["--pipe", &good_pipe, "--token", "short", "--parent-pid", "1"]),
            s(&["--pipe", &good_pipe, "--token", &tok, "--parent-pid", "x"]),
            s(&["--pipe", &good_pipe, "--token", &tok]),
            s(&["--pipe", &good_pipe, "--token", &tok, "--parent-pid", "1", "--evil", "x"]),
            s(&["--pipe", &format!("{good_pipe}&calc"), "--token", &tok, "--parent-pid", "1"]),
        ] {
            assert!(HelperArgs::parse(bad.clone()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn unknown_ops_rejected() {
        let r: Result<AppMessage, _> =
            serde_json::from_str(r#"{"type":"request","id":1,"op":{"op":"runCommand","cmd":"x"}}"#);
        assert!(r.is_err());
        let r: Result<AppMessage, _> =
            serde_json::from_str(r#"{"type":"request","id":1,"op":{"op":"ping","nonce":5,"extra":1}}"#);
        assert!(r.is_err());
        let ok: AppMessage =
            serde_json::from_str(r#"{"type":"request","id":1,"op":{"op":"ping","nonce":5}}"#).unwrap();
        assert_eq!(ok, AppMessage::Request { id: 1, op: HelperOp::Ping { nonce: 5 } });
    }

    #[test]
    fn token_compare() {
        assert!(token_eq("abc", "abc"));
        assert!(!token_eq("abc", "abd"));
        assert!(!token_eq("abc", "abcd"));
    }
}
