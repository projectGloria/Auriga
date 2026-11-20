//! Typed form of a KB rule file. Mirrors `kb/schema/rule.schema.json`; the
//! schema gives readable errors, these types give the evaluator its input.
//! Regexes are compiled while deserialising, so a bad pattern is a load error.

use crate::analyze::{Level, SignalKind};
use crate::snapshot::SignatureStatus;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Severity {
    Info,
    Warning,
    Problem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Device,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct SourceRef {
    pub title: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

/// Case-insensitive regexes; matches when any one matches.
#[derive(Debug, Clone)]
pub struct Patterns(Vec<Regex>);

impl Patterns {
    pub fn any(&self, s: &str) -> bool {
        self.0.iter().any(|r| r.is_match(s))
    }
}

impl<'de> Deserialize<'de> for Patterns {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = Vec::<String>::deserialize(d)?;
        raw.iter()
            .map(|p| {
                RegexBuilder::new(p)
                    .case_insensitive(true)
                    .size_limit(1 << 20)
                    .build()
                    .map_err(|e| serde::de::Error::custom(format!("invalid pattern {p:?}: {e}")))
            })
            .collect::<Result<_, _>>()
            .map(Patterns)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub version: u32,
    pub title: String,
    pub severity: Severity,
    pub confidence: Confidence,
    #[serde(default)]
    pub confidence_note: Option<String>,
    pub category: String,
    pub scope: Scope,
    #[serde(default, rename = "match")]
    pub selector: Option<Selector>,
    #[serde(default)]
    pub when: Option<Condition>,
    pub summary: String,
    pub explanation: String,
    pub sources: Vec<SourceRef>,
    #[serde(default)]
    pub fixes: Vec<FixDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selector {
    #[serde(default)]
    pub class: Option<Vec<String>>,
    #[serde(default)]
    pub hardware_id: Option<Patterns>,
    #[serde(default)]
    pub exclude_hardware_id: Option<Patterns>,
    #[serde(default)]
    pub compatible_id: Option<Patterns>,
    #[serde(default)]
    pub service: Option<Patterns>,
    #[serde(default)]
    pub enumerator: Option<Vec<String>>,
    #[serde(default)]
    pub driver_provider: Option<Patterns>,
    #[serde(default)]
    pub include_not_present: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRef {
    #[default]
    #[serde(rename = "self")]
    This,
    Any,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    #[serde(default = "one")]
    pub min: u32,
    #[serde(default)]
    pub days: Option<u32>,
    #[serde(default)]
    pub device: DeviceRef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalCond {
    pub kind: SignalKind,
    #[serde(default = "one")]
    pub min: u32,
    #[serde(default)]
    pub days: Option<u32>,
    #[serde(default)]
    pub device: DeviceRef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventCond {
    pub provider: Patterns,
    #[serde(default)]
    pub ids: Option<Vec<u32>>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub max_level: Option<u8>,
    #[serde(default)]
    pub data: std::collections::BTreeMap<String, String>,
    #[serde(default = "one")]
    pub min: u32,
    #[serde(default)]
    pub days: Option<u32>,
    #[serde(default)]
    pub device: DeviceRef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyCond {
    pub keyword: String,
    #[serde(default)]
    pub equals: Option<String>,
    #[serde(default)]
    pub not_equals: Option<String>,
    #[serde(default, rename = "in")]
    pub one_of: Option<Vec<String>>,
    #[serde(default)]
    pub not_in: Option<Vec<String>>,
    #[serde(default)]
    pub exists: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerCond {
    pub allow_turn_off: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriverCond {
    #[serde(default)]
    pub provider: Option<Patterns>,
    #[serde(default)]
    pub inbox: Option<bool>,
    #[serde(default)]
    pub version_below: Option<String>,
    #[serde(default)]
    pub version_at_least: Option<String>,
    #[serde(default)]
    pub signature: Option<Vec<SignatureStatus>>,
    #[serde(default)]
    pub older_than_days: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DumpCond {
    #[serde(default = "one")]
    pub min: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Not(Box<Condition>),
    Signal(SignalCond),
    Event(EventCond),
    Property(PropertyCond),
    ProblemCode(Vec<u32>),
    Power(PowerCond),
    Driver(DriverCond),
    MatchLevel(Vec<Level>),
    DevicePresent(Selector),
    SetupapiFailure(Window),
    DumpFiles(DumpCond),
}

impl Condition {
    /// Every sub-condition, depth first (including `self`).
    pub fn walk(&self, f: &mut dyn FnMut(&Condition)) {
        f(self);
        match self {
            Condition::All(v) | Condition::Any(v) => v.iter().for_each(|c| c.walk(f)),
            Condition::Not(c) => c.walk(f),
            _ => {}
        }
    }

    /// True for conditions that read the matched device.
    pub fn needs_device(&self) -> bool {
        match self {
            Condition::Signal(s) => s.device == DeviceRef::This,
            Condition::Event(e) => e.device == DeviceRef::This,
            Condition::SetupapiFailure(w) => w.device == DeviceRef::This,
            Condition::Property(_)
            | Condition::ProblemCode(_)
            | Condition::Power(_)
            | Condition::Driver(_)
            | Condition::MatchLevel(_) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixDef {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default)]
    pub tradeoff: Option<String>,
    pub risk: Risk,
    #[serde(default)]
    pub optional: bool,
    pub action: FixAction,
}

/// What a fix does. Only these typed actions exist; the fix engine and the
/// elevated helper accept nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FixAction {
    AdapterProperty {
        keyword: String,
        value: String,
    },
    DevicePower {
        allow_turn_off: bool,
    },
    RollbackDriver {},
    Guided {
        steps: Vec<String>,
        #[serde(default)]
        page: Option<String>,
    },
}

impl FixAction {
    pub fn requires_admin(&self) -> bool {
        !matches!(self, FixAction::Guided { .. })
    }
}
