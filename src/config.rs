use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Canonical display order of the built-in kinds, used when a config omits
/// `order`. Must mirror the registry in `sources::all_sources()`.
pub const DEFAULT_KIND_ORDER: [&str; 14] = [
    "os", "kernel", "host", "hostname", "user", "shell", "terminal", "wm", "uptime", "cpu", "gpu",
    "memory", "disk", "locale",
];

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub commit: CommitConfig,
    #[serde(default)]
    pub messages: MessagesConfig,
    #[serde(default)]
    pub sections: BTreeMap<String, SectionEntry>,
    /// Exact display list: only listed sections render, in listed order.
    /// Absent means "every defined section, canonical-then-alphabetical".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CommitConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_email: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MessagesConfig {
    #[serde(default)]
    pub pool: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum FieldValue {
    Value(String),
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fallback: Option<String>,
    },
}

/// One named entry in `sections`. The map key doubles as the identity: for a
/// co-author it selects the built-in source (or marks a config-only entry).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SectionEntry {
    /// Static text body line, printed verbatim; an empty string is a blank
    /// line (the `blank_line_before` replacement).
    TextLine(String),
    /// Text line with `{placeholder}` templates filled from its `fields`.
    /// Discriminated from a co-author by the required `text` key.
    TextField(TextField),
    CoAuthor(CoAuthorConfig),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TextField {
    pub text: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, FieldValue>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CoAuthorConfig {
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<FieldValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<FieldValue>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, FieldValue>,
}

fn default_true() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

impl Config {
    pub fn load(explicit: Option<&Path>) -> Config {
        let mut cfg: Config = serde_json::from_str(include_str!("default-config.json"))
            .expect("embedded default config must parse");

        let path = explicit.map(PathBuf::from).or_else(default_config_path);

        if let Some(p) = explicit
            && !p.exists()
        {
            eprintln!(
                "blamefetch: warning: config file {} does not exist; using defaults",
                p.display()
            );
        }

        if let Some(path) = path.filter(|p| p.exists()) {
            match std::fs::read_to_string(&path) {
                Ok(text) => match serde_json::from_str::<Config>(&text) {
                    Ok(user) => cfg.merge(user),
                    Err(err) => eprintln!(
                        "blamefetch: warning: failed to parse config {}: {err}; using defaults",
                        path.display()
                    ),
                },
                Err(err) => eprintln!(
                    "blamefetch: warning: cannot read config {}: {err}; using defaults",
                    path.display()
                ),
            }
        }
        cfg
    }

    /// The effective config as JSON, with `order` materialized to the resolved
    /// display list, so `--print-config` shows exactly what would render.
    pub fn to_json(&self) -> String {
        let mut c = self.clone();
        c.order = Some(
            self.ordered_sections()
                .into_iter()
                .map(|(k, _)| k)
                .collect(),
        );
        serde_json::to_string_pretty(&c).expect("config must serialize")
    }

    /// Sections in display order:
    /// - explicit `order`: exactly the listed keys, in listed order; keys not
    ///   defined in `sections` are warned about and skipped;
    /// - absent `order`: built-in kinds in canonical order, then custom keys
    ///   alphabetically (`BTreeMap` iteration guarantees determinism).
    pub fn ordered_sections(&self) -> Vec<(String, &SectionEntry)> {
        match &self.order {
            Some(order) => order
                .iter()
                .filter_map(|key| match self.sections.get(key) {
                    Some(entry) => Some((key.clone(), entry)),
                    None => {
                        eprintln!(
                            "blamefetch: warning: order references undefined section {key:?}; skipping"
                        );
                        None
                    }
                })
                .collect(),
            None => {
                let mut out = Vec::new();
                for kind in DEFAULT_KIND_ORDER {
                    if let Some(entry) = self.sections.get(kind) {
                        out.push((kind.to_string(), entry));
                    }
                }
                for (key, entry) in &self.sections {
                    if !DEFAULT_KIND_ORDER.contains(&key.as_str()) {
                        out.push((key.clone(), entry));
                    }
                }
                out
            }
        }
    }

    fn merge(&mut self, user: Config) {
        if let Some(name) = user.commit.author_name {
            self.commit.author_name = Some(name);
        }
        if let Some(email) = user.commit.author_email {
            self.commit.author_email = Some(email);
        }
        if !user.messages.pool.is_empty() {
            self.messages.pool = user.messages.pool;
        }
        if !user.sections.is_empty() {
            self.sections = user.sections;
            // Sections are the user's now; the embedded default order no
            // longer applies unless the user supplied their own.
            self.order = user.order;
        } else if let Some(order) = user.order {
            self.order = Some(order);
        }
    }
}

fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("blamefetch").join("config.json"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde::Deserialize;

    use super::{
        CoAuthorConfig, CommitConfig, Config, DEFAULT_KIND_ORDER, FieldValue, MessagesConfig,
        SectionEntry,
    };

    fn base() -> Config {
        // A path that cannot exist: Config::load warns and falls back to the
        // embedded defaults, so the developer's real config
        // (~/.config/blamefetch/config.json) cannot leak into the base.
        let missing =
            std::env::temp_dir().join(format!("blamefetch-no-such-{}", std::process::id()));
        Config::load(Some(&missing))
    }

    #[test]
    fn defaults_parse_with_roster_and_pool() {
        let c = base();
        assert!(!c.messages.pool.is_empty());
        assert_eq!(c.sections.len(), DEFAULT_KIND_ORDER.len());
        for kind in DEFAULT_KIND_ORDER {
            assert!(
                matches!(c.sections.get(kind), Some(SectionEntry::CoAuthor(_))),
                "missing built-in section {kind}"
            );
        }
    }

    #[test]
    fn default_order_lists_all_kinds_in_canonical_order() {
        let c = base();
        let order = c.order.clone().expect("default config carries an order");
        assert_eq!(order, DEFAULT_KIND_ORDER);
    }

    #[test]
    fn user_pool_replaces_default() {
        let mut c = base();
        let user = Config {
            commit: CommitConfig::default(),
            messages: MessagesConfig {
                pool: vec!["mine".to_string()],
            },
            sections: BTreeMap::new(),
            order: None,
        };
        c.merge(user);
        assert_eq!(c.messages.pool, vec!["mine".to_string()]);
        assert!(!c.sections.is_empty(), "sections must stay from defaults");
    }

    #[test]
    fn user_sections_replace_default_roster() {
        let mut c = base();
        let user = Config {
            commit: CommitConfig::default(),
            messages: MessagesConfig { pool: vec![] },
            sections: BTreeMap::from([(
                "only".to_string(),
                SectionEntry::TextLine("hi".to_string()),
            )]),
            order: None,
        };
        c.merge(user);
        assert_eq!(c.sections.len(), 1);
        assert!(matches!(
            c.sections.get("only"),
            Some(SectionEntry::TextLine(s)) if s == "hi"
        ));
    }

    #[test]
    fn user_sections_without_order_clear_default_order() {
        let mut c = base();
        let user = Config {
            commit: CommitConfig::default(),
            messages: MessagesConfig { pool: vec![] },
            sections: BTreeMap::from([(
                "bot".to_string(),
                SectionEntry::CoAuthor(CoAuthorConfig {
                    enabled: true,
                    name: Some(FieldValue::Value("Bot".to_string())),
                    email: Some(FieldValue::Value("bot@x.com".to_string())),
                    fields: BTreeMap::new(),
                }),
            )]),
            order: None,
        };
        c.merge(user);
        let listed = c.ordered_sections();
        let keys: Vec<&str> = listed.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            ["bot"],
            "default order must not filter out user-defined sections"
        );
    }

    #[test]
    fn user_order_replaces_default_order() {
        let mut c = base();
        let user = Config {
            commit: CommitConfig::default(),
            messages: MessagesConfig { pool: vec![] },
            sections: BTreeMap::new(),
            order: Some(vec!["kernel".to_string(), "os".to_string()]),
        };
        c.merge(user);
        assert_eq!(
            c.order,
            Some(vec!["kernel".to_string(), "os".to_string()]),
            "order must replace, not append"
        );
    }

    #[test]
    fn commit_fields_merge_fieldwise() {
        // Load from a minimal valid JSON config so the developer's real config
        // (~/.config/blamefetch/config.json) cannot leak into the base.
        let dir = std::env::temp_dir().join(format!("blamefetch-cfg-merge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.json");
        std::fs::write(&path, "{}").unwrap();
        let mut c = Config::load(Some(&path));
        std::fs::remove_dir_all(&dir).unwrap();
        let user = Config {
            commit: CommitConfig {
                author_name: Some("A".to_string()),
                author_email: None,
            },
            messages: MessagesConfig { pool: vec![] },
            sections: BTreeMap::new(),
            order: None,
        };
        c.merge(user);
        assert_eq!(c.commit.author_name.as_deref(), Some("A"));
        assert!(
            c.commit.author_email.is_none(),
            "unset field must stay None"
        );
    }

    #[test]
    fn bad_config_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("blamefetch-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.json");
        std::fs::write(&path, "{ this is not json").unwrap();
        let c = Config::load(Some(&path));
        assert!(!c.sections.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn to_json_roundtrips() {
        let c = base();
        let s = c.to_json();
        let parsed: Config = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.sections.len(), c.sections.len());
        assert_eq!(
            parsed.order,
            Some(DEFAULT_KIND_ORDER.iter().map(|k| k.to_string()).collect()),
            "to_json must materialize the resolved display order"
        );
    }

    #[test]
    fn field_value_untagged_parses() {
        #[derive(Deserialize)]
        struct Wrap {
            v: FieldValue,
        }
        let s: Wrap = serde_json::from_str(r#"{"v": "literal"}"#).unwrap();
        assert_eq!(s.v, FieldValue::Value("literal".to_string()));

        let c: Wrap =
            serde_json::from_str(r#"{"v": {"command": "sample --version | cut -d' ' -f1"}}"#)
                .unwrap();
        assert_eq!(
            c.v,
            FieldValue::Command {
                command: "sample --version | cut -d' ' -f1".to_string(),
                fallback: None,
            }
        );

        let f: Wrap = serde_json::from_str(r#"{"v": {"command": "x", "fallback": "y"}}"#).unwrap();
        assert_eq!(
            f.v,
            FieldValue::Command {
                command: "x".to_string(),
                fallback: Some("y".to_string()),
            }
        );
    }

    #[test]
    fn command_serialization_roundtrips() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "version".to_string(),
            FieldValue::Command {
                command: "sample --version | cut -d' ' -f1".to_string(),
                fallback: Some("unknown".to_string()),
            },
        );
        let cfg = Config {
            commit: CommitConfig::default(),
            messages: MessagesConfig { pool: vec![] },
            sections: BTreeMap::from([(
                "sample".to_string(),
                SectionEntry::CoAuthor(CoAuthorConfig {
                    enabled: true,
                    name: Some(FieldValue::Value("Sample {version}".to_string())),
                    email: Some(FieldValue::Value("noreply@example.com".to_string())),
                    fields,
                }),
            )]),
            order: Some(vec!["sample".to_string()]),
        };
        let s = cfg.to_json();
        let parsed: Config = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.sections.len(), 1);
        let ca = match parsed.sections.get("sample") {
            Some(SectionEntry::CoAuthor(ca)) => ca,
            _ => panic!("expected CoAuthor"),
        };
        assert!(ca.enabled);
        match &ca.fields["version"] {
            FieldValue::Command { command, fallback } => {
                assert_eq!(command, "sample --version | cut -d' ' -f1");
                assert_eq!(fallback.as_deref(), Some("unknown"));
            }
            _ => panic!("expected Command"),
        }
    }

    #[test]
    fn section_entry_untagged_parses_three_shapes() {
        let c: Config = serde_json::from_str(
            r#"{
                "sections": {
                    "note": "plain text",
                    "dynamic": { "text": "Hello {name}" },
                    "bot": { "name": "Only", "email": "only@x.com" }
                }
            }"#,
        )
        .unwrap();
        assert!(matches!(
            c.sections.get("note"),
            Some(SectionEntry::TextLine(s)) if s == "plain text"
        ));
        match c.sections.get("dynamic") {
            Some(SectionEntry::TextField(tf)) => {
                assert_eq!(tf.text, "Hello {name}");
                assert!(tf.fields.is_empty());
            }
            _ => panic!("expected TextField"),
        }
        match c.sections.get("bot") {
            Some(SectionEntry::CoAuthor(ca)) => {
                assert!(ca.enabled);
                assert_eq!(ca.name, Some(FieldValue::Value("Only".to_string())));
            }
            _ => panic!("expected CoAuthor"),
        }
    }

    #[test]
    fn ordered_sections_explicit_order_is_exact_display_list() {
        let c: Config = serde_json::from_str(
            r#"{
                "sections": {
                    "a": "first",
                    "b": { "text": "second" },
                    "c": { "name": "c", "email": "c@x.com" }
                },
                "order": ["c", "a"]
            }"#,
        )
        .unwrap();
        let listed = c.ordered_sections();
        let order: Vec<&str> = listed.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(order, ["c", "a"], "unlisted section b must not render");
    }

    #[test]
    fn ordered_sections_missing_key_warns_and_skips() {
        let c: Config =
            serde_json::from_str(r#"{"sections": {"a": "x"}, "order": ["nope", "a"]}"#).unwrap();
        let listed = c.ordered_sections();
        let order: Vec<&str> = listed.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(order, ["a"]);
    }

    #[test]
    fn ordered_sections_without_order_uses_canonical_then_alpha() {
        let c: Config = serde_json::from_str(
            r#"{
                "sections": {
                    "zeta": { "name": "z", "email": "z@x.com" },
                    "os": { "name": "OS" },
                    "kernel": { "name": "K" },
                    "alpha": { "name": "a", "email": "a@x.com" }
                }
            }"#,
        )
        .unwrap();
        let listed = c.ordered_sections();
        let order: Vec<&str> = listed.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(order, ["os", "kernel", "alpha", "zeta"]);
    }

    #[test]
    fn canonical_kind_order_matches_source_registry() {
        let kinds: Vec<&str> = crate::sources::all_sources()
            .iter()
            .map(|s| s.kind())
            .collect();
        assert_eq!(kinds, DEFAULT_KIND_ORDER);
    }
}
