//! Loads `kb/rules/*.yaml`: YAML → JSON value → JSON Schema → typed `Rule`
//! → semantic checks. A bad file is reported and skipped; the rest load.

use super::rule::{Condition, Rule, Scope};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use ts_rs::TS;

/// The schema the Rust types implement. Embedded (not read from disk) so an
/// edited schema file can never disagree with the evaluator.
pub const RULE_SCHEMA: &str = include_str!("../../../../kb/schema/rule.schema.json");

#[derive(Debug, Clone)]
pub struct LoadedRule {
    pub rule: Rule,
    /// File name relative to the rules directory.
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KbError {
    pub file: String,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct Kb {
    pub dir: Option<PathBuf>,
    pub kb_version: Option<String>,
    pub rules: Vec<LoadedRule>,
    pub errors: Vec<KbError>,
    /// RFC 3339.
    pub loaded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KbStatus {
    pub dir: Option<String>,
    pub kb_version: Option<String>,
    pub rule_count: u32,
    pub errors: Vec<KbError>,
    pub loaded_at: String,
}

impl Kb {
    pub fn status(&self) -> KbStatus {
        KbStatus {
            dir: self.dir.as_ref().map(|d| d.display().to_string()),
            kb_version: self.kb_version.clone(),
            rule_count: self.rules.len() as u32,
            errors: self.errors.clone(),
            loaded_at: self.loaded_at.clone(),
        }
    }

    /// A KB that could not be found at all.
    pub fn missing(reason: String) -> Self {
        Kb {
            errors: vec![KbError {
                file: String::new(),
                message: reason,
            }],
            loaded_at: chrono::Utc::now().to_rfc3339(),
            ..Default::default()
        }
    }
}

fn validator() -> &'static jsonschema::Validator {
    static V: OnceLock<jsonschema::Validator> = OnceLock::new();
    V.get_or_init(|| {
        let schema: serde_json::Value =
            serde_json::from_str(RULE_SCHEMA).expect("embedded rule schema is valid JSON");
        jsonschema::validator_for(&schema).expect("embedded rule schema compiles")
    })
}

/// Parse and check one rule file's text.
pub fn parse_rule(text: &str) -> Result<Rule, String> {
    let value: serde_json::Value =
        serde_norway::from_str(text).map_err(|e| format!("YAML: {e}"))?;
    let errs: Vec<String> = validator()
        .iter_errors(&value)
        .take(5)
        .map(|e| {
            let path = e.instance_path().to_string();
            if path.is_empty() {
                e.to_string()
            } else {
                format!("{path}: {e}")
            }
        })
        .collect();
    if !errs.is_empty() {
        return Err(format!("schema: {}", errs.join("; ")));
    }
    let rule: Rule = serde_json::from_value(value).map_err(|e| format!("rule: {e}"))?;
    check_semantics(&rule)?;
    Ok(rule)
}

/// Checks the schema cannot express.
fn check_semantics(r: &Rule) -> Result<(), String> {
    let mut ids = HashSet::new();
    for f in &r.fixes {
        if !ids.insert(f.id.as_str()) {
            return Err(format!("duplicate fix id {:?}", f.id));
        }
    }
    if r.scope == Scope::System {
        let mut bad = None;
        if let Some(c) = &r.when {
            c.walk(&mut |c: &Condition| {
                if c.needs_device() {
                    bad.get_or_insert(());
                }
            });
        }
        if bad.is_some() {
            return Err(
                "system-scope rule uses a device condition (property, power, driver, problem_code, match_level, or device: self)".into(),
            );
        }
        if r.fixes.iter().any(|f| f.action.requires_admin()) {
            return Err("system-scope rule can only have guided fixes".into());
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct Manifest {
    kb_version: Option<String>,
}

/// Load every `*.yaml`/`*.yml` in `<dir>/rules`, sorted by file name.
pub fn load_dir(dir: &Path) -> Kb {
    let mut kb = Kb {
        dir: Some(dir.to_path_buf()),
        loaded_at: chrono::Utc::now().to_rfc3339(),
        ..Default::default()
    };
    // The manifest is optional; a missing file just leaves kb_version unset.
    if let Ok(t) = std::fs::read_to_string(dir.join("manifest.yaml")) {
        match serde_norway::from_str::<Manifest>(&t) {
            Ok(m) => kb.kb_version = m.kb_version,
            Err(e) => kb.errors.push(KbError {
                file: "manifest.yaml".into(),
                message: format!("YAML: {e}"),
            }),
        }
    }
    let rules_dir = dir.join("rules");
    let mut files: Vec<PathBuf> = match std::fs::read_dir(&rules_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.is_file()
                    && p.extension().and_then(|x| x.to_str()).is_some_and(|x| {
                        x.eq_ignore_ascii_case("yaml") || x.eq_ignore_ascii_case("yml")
                    })
            })
            .collect(),
        Err(e) => {
            kb.errors.push(KbError {
                file: "rules".into(),
                message: format!("cannot read {}: {e}", rules_dir.display()),
            });
            return kb;
        }
    };
    files.sort();
    let mut seen: HashSet<String> = HashSet::new();
    for path in files {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                kb.errors.push(KbError {
                    file,
                    message: format!("cannot read: {e}"),
                });
                continue;
            }
        };
        match parse_rule(&text) {
            Ok(rule) if !seen.insert(rule.id.clone()) => kb.errors.push(KbError {
                file,
                message: format!("duplicate rule id {:?}; this file was skipped", rule.id),
            }),
            Ok(rule) => kb.rules.push(LoadedRule { rule, file }),
            Err(message) => kb.errors.push(KbError { file, message }),
        }
    }
    kb
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: &str = r#"
id: test.rule
version: 1
title: Test rule
severity: info
confidence: low
category: other
scope: device
match: { class: [Net] }
when: { problem_code: [] }
summary: s
explanation: e
sources: []
"#;

    #[test]
    fn minimal_rule_parses() {
        let r = parse_rule(MIN).unwrap();
        assert_eq!(r.id, "test.rule");
    }

    #[test]
    fn schema_rejects_unknown_field_and_bad_enum() {
        let e = parse_rule(&MIN.replace("severity: info", "severity: critical")).unwrap_err();
        assert!(e.starts_with("schema:"), "{e}");
        let e = parse_rule(&format!("{MIN}colour: red\n")).unwrap_err();
        assert!(e.starts_with("schema:"), "{e}");
    }

    #[test]
    fn schema_requires_match_for_device_scope() {
        let e = parse_rule(&MIN.replace("match: { class: [Net] }\n", "")).unwrap_err();
        assert!(e.contains("match"), "{e}");
    }

    #[test]
    fn bad_regex_is_a_load_error() {
        let e = parse_rule(&MIN.replace("class: [Net]", "hardware_id: ['(unclosed']")).unwrap_err();
        assert!(e.contains("invalid pattern"), "{e}");
    }

    #[test]
    fn system_rule_cannot_use_device_conditions() {
        let t = MIN
            .replace("scope: device", "scope: system")
            .replace("match: { class: [Net] }\n", "");
        let e = parse_rule(&t).unwrap_err();
        assert!(e.contains("device condition"), "{e}");
    }

    #[test]
    fn unknown_fix_action_rejected() {
        let t = format!(
            "{MIN}fixes:\n  - id: x\n    title: x\n    risk: low\n    action: {{ run_command: {{ cmd: 'reg add' }} }}\n"
        );
        assert!(parse_rule(&t).unwrap_err().starts_with("schema:"));
    }

    #[test]
    fn dir_load_skips_bad_files_and_duplicates() {
        let d = tempfile::tempdir().unwrap();
        let rules = d.path().join("rules");
        std::fs::create_dir(&rules).unwrap();
        std::fs::write(rules.join("a.yaml"), MIN).unwrap();
        std::fs::write(rules.join("b.yaml"), MIN).unwrap();
        std::fs::write(rules.join("c.yaml"), "id: [").unwrap();
        std::fs::write(rules.join("notes.txt"), "ignored").unwrap();
        let kb = load_dir(d.path());
        assert_eq!(kb.rules.len(), 1);
        assert_eq!(kb.errors.len(), 2);
        assert!(
            kb.errors
                .iter()
                .any(|e| e.file == "b.yaml" && e.message.contains("duplicate"))
        );
        assert!(
            kb.errors
                .iter()
                .any(|e| e.file == "c.yaml" && e.message.starts_with("YAML"))
        );
    }
}
