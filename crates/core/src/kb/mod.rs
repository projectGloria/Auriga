//! Offline knowledge base: rule files in `kb/rules`, validated against
//! `kb/schema/rule.schema.json`, evaluated purely over a snapshot.

pub mod eval;
pub mod loader;
pub mod rule;
pub mod watch;

pub use eval::{eval, eval_snapshot};
pub use loader::{Kb, KbError, KbStatus, LoadedRule, load_dir, parse_rule};
pub use rule::{Confidence, FixAction, Risk, Rule, Severity, SourceRef};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One rule that fired, for one device (or for the system).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Finding {
    /// Stable across scans: `rule_id` or `rule_id@instance_id`.
    pub key: String,
    pub rule_id: String,
    pub rule_version: u32,
    pub title: String,
    pub severity: Severity,
    pub confidence: Confidence,
    pub confidence_note: Option<String>,
    pub category: String,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    /// `{device}` replaced with the device name.
    pub summary: String,
    pub explanation: String,
    /// Why the rule fired, from this machine's data.
    pub evidence: Vec<String>,
    pub sources: Vec<SourceRef>,
    pub fixes: Vec<FixSuggestion>,
    /// Rule file, relative to `kb/rules`.
    pub source_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FixSuggestion {
    pub id: String,
    pub title: String,
    pub why: Option<String>,
    pub tradeoff: Option<String>,
    pub risk: Risk,
    pub optional: bool,
    pub requires_admin: bool,
    pub action: FixAction,
    /// Checked against the device's *current* properties in the snapshot.
    pub status: FixStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "state")]
#[ts(export)]
pub enum FixStatus {
    /// Would change `from` → `to`. `from` is `None` when the current value is unknown.
    Applicable {
        from: Option<ValueText>,
        to: ValueText,
    },
    /// The device already has this value.
    AlreadySet { value: ValueText },
    /// Cannot be applied on this machine (e.g. the driver lacks the keyword).
    Unavailable { reason: String },
    /// Steps for the user; nothing is changed automatically.
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ValueText {
    /// Raw value, e.g. `"3"`.
    pub value: String,
    /// Driver's label for it, e.g. "No SMPS".
    pub label: Option<String>,
}

/// What the Findings page needs in one call.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FindingsReport {
    pub kb: KbStatus,
    /// `None` when there is no scan yet.
    pub scan_collected_at: Option<String>,
    pub findings: Vec<Finding>,
}
