use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Canonical display order of the built-in kinds, used when a config omits
/// `order`. Derived from the source registry itself so the two cannot drift
/// apart.
pub fn canonical_kind_order() -> Vec<&'static str> {
    crate::sources::all_sources()
        .into_iter()
        .map(|s| s.kind())
        .collect()
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
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

/// Top-level config keys. The single source of truth shared by the
/// unknown-key walk and the tolerant per-key parse, so a new field cannot
/// drift between the two.
const TOP_LEVEL_KEYS: &[&str] = &["commit", "messages", "sections", "order"];

/// Deserializes one top-level config key with tolerance: a malformed value
/// drops only that key, with a warning naming the JSON path.
fn parse_key<T: serde::de::DeserializeOwned>(
    v: serde_json::Value,
    label: &str,
    apply: impl FnOnce(T),
) {
    match serde_json::from_value::<T>(v) {
        Ok(value) => apply(value),
        Err(e) => eprintln!("blamefetch: warning: ignoring invalid {label:?} entry: {e}"),
    }
}

/// Parses the `sections` map with per-entry tolerance: a malformed entry
/// drops only that entry.
fn parse_sections(v: serde_json::Value, user: &mut Config) {
    let serde_json::Value::Object(sections) = v else {
        eprintln!("blamefetch: warning: ignoring invalid \"sections\" (must be an object)");
        return;
    };
    for (key, entry) in sections {
        match serde_json::from_value::<SectionEntry>(entry) {
            Ok(e) => {
                user.sections.insert(key, e);
            }
            Err(e) => eprintln!("blamefetch: warning: ignoring invalid section {key:?}: {e}"),
        }
    }
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
                Ok(text) => {
                    // Parse the file exactly once: the unknown-key walk and the
                    // tolerant per-key parse share the same `Value`.
                    match serde_json::from_str(&text) {
                        Ok(value) => {
                            report_unknown_keys(&value);
                            match Self::parse_user_value(value) {
                                Ok(user) => cfg.merge(user),
                                Err(err) => eprintln!(
                                    "blamefetch: warning: {err} (in {}); using defaults",
                                    path.display()
                                ),
                            }
                        }
                        Err(err) => eprintln!(
                            "blamefetch: warning: failed to parse config: {err} (in {}); using defaults",
                            path.display()
                        ),
                    }
                }
                Err(err) => eprintln!(
                    "blamefetch: warning: cannot read config {}: {err}; using defaults",
                    path.display()
                ),
            }
        }
        cfg
    }

    /// Parses a user config value with per-key tolerance: a malformed
    /// `sections` entry only drops that entry, and a malformed top-level key
    /// only drops that key — each with a warning naming the JSON path —
    /// instead of discarding the whole file over one typo. Only a JSON syntax
    /// error (or a non-object document) rejects the file outright.
    fn parse_user_value(value: serde_json::Value) -> Result<Config, String> {
        let serde_json::Value::Object(mut obj) = value else {
            return Err("config root must be a JSON object".to_string());
        };
        let mut user = Config::default();

        // One dispatch over the shared key table: `obj.remove` hands the
        // owned value to the parser, so nothing is deep-cloned.
        for key in TOP_LEVEL_KEYS {
            let Some(v) = obj.remove(*key) else {
                continue;
            };
            match *key {
                "commit" => parse_key(v, "commit", |c| user.commit = c),
                "messages" => parse_key(v, "messages", |m| user.messages = m),
                // `Option` keeps `"order": null` equivalent to an absent
                // order (serde's old whole-file behavior), while
                // `"order": []` stays an explicit empty display list.
                "order" => parse_key(v, "order", |o| user.order = o),
                "sections" => parse_sections(v, &mut user),
                _ => unreachable!("TOP_LEVEL_KEYS is exhaustive"),
            }
        }
        Ok(user)
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
                let canonical = canonical_kind_order();
                let mut out: Vec<(String, &SectionEntry)> = canonical
                    .iter()
                    .filter_map(|kind| {
                        self.sections
                            .get(*kind)
                            .map(|entry| (kind.to_string(), entry))
                    })
                    .collect();
                for (key, entry) in &self.sections {
                    if !canonical.contains(&key.as_str()) {
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
        // Per-key: a user section overrides the built-in of the same key;
        // everything else stays. To hide a built-in, disable it with
        // `enabled: false`; to show only a few sections, list them in `order`.
        for (key, entry) in user.sections {
            self.sections.insert(key, entry);
        }
        // An explicit user order replaces; absent keeps the derived default.
        if let Some(order) = user.order {
            self.order = Some(order);
        }
    }
}

fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("blamefetch").join("config.json"))
}

/// Warns about unrecognized keys in a config file. serde's untagged enums
/// (`SectionEntry`, `FieldValue`) make `deny_unknown_fields` unusable — it
/// would reject the whole config with a generic error — so a plain value walk
/// flags each offending key path without failing the parse. Consumes the
/// already-parsed `Value`; a JSON syntax error is reported by the caller.
fn report_unknown_keys(value: &serde_json::Value) {
    let unknown = unknown_keys(value);
    if !unknown.is_empty() {
        eprintln!(
            "blamefetch: warning: unrecognized config key(s) ignored: {}",
            unknown.join(", ")
        );
    }
}

/// Recognized keys per level. `BTreeSet` keeps the result sorted and deduped
/// so the warning is deterministic.
fn unknown_keys(value: &serde_json::Value) -> Vec<String> {
    let mut out = BTreeSet::new();
    check_object(Some(value), "", TOP_LEVEL_KEYS, &mut out);
    check_object(
        value.get("commit"),
        "commit",
        &["author_name", "author_email"],
        &mut out,
    );
    check_object(value.get("messages"), "messages", &["pool"], &mut out);

    if let Some(sections) = value.get("sections").and_then(|v| v.as_object()) {
        for (key, entry) in sections {
            let base = format!("sections.{key}");
            let Some(obj) = entry.as_object() else {
                continue; // a string is a TextLine
            };
            // An object with a `text` key is a TextField shape; without one it
            // is a co-author shape (serde's untagged rule).
            let allowed = if obj.contains_key("text") {
                &["text", "fields"][..]
            } else {
                &["enabled", "name", "email", "fields"][..]
            };
            check_object(Some(entry), &base, allowed, &mut out);
            if let Some(fields) = obj.get("fields") {
                check_fields(fields, &format!("{base}.fields"), &mut out);
            }
            if !obj.contains_key("text") {
                // A co-author's `name`/`email` may be command objects; flag
                // typos inside them just like in `fields`.
                for field in ["name", "email"] {
                    if let Some(fv) = obj.get(field).filter(|v| v.is_object()) {
                        check_object(
                            Some(fv),
                            &format!("{base}.{field}"),
                            &["command", "fallback"],
                            &mut out,
                        );
                    }
                }
            }
        }
    }
    out.into_iter().collect()
}

fn check_fields(value: &serde_json::Value, base: &str, out: &mut BTreeSet<String>) {
    let Some(map) = value.as_object() else {
        return; // `fields` must be an object; serde will reject it otherwise
    };
    for (key, fv) in map {
        let path = format!("{base}.{key}");
        if fv.is_object() {
            check_object(Some(fv), &path, &["command", "fallback"], out);
        }
    }
}

fn check_object(
    value: Option<&serde_json::Value>,
    base: &str,
    allowed: &[&str],
    out: &mut BTreeSet<String>,
) {
    let Some(map) = value.and_then(|v| v.as_object()) else {
        return;
    };
    for key in map.keys() {
        let path = if base.is_empty() {
            key.clone()
        } else {
            format!("{base}.{key}")
        };
        if !allowed.contains(&key.as_str()) {
            out.insert(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use serde::Deserialize;

    use super::{
        CoAuthorConfig, CommitConfig, Config, FieldValue, MessagesConfig, SectionEntry,
        canonical_kind_order,
    };

    fn base() -> Config {
        // A path that cannot exist: Config::load warns and falls back to the
        // embedded defaults, so the developer's real config
        // (~/.config/blamefetch/config.json) cannot leak into the base.
        let missing =
            std::env::temp_dir().join(format!("blamefetch-no-such-{}", std::process::id()));
        Config::load(Some(&missing))
    }

    /// Loads a config from literal text through a real temp file. The
    /// `TempDir` is scoped to this helper: it is removed when the helper
    /// returns (or unwinds on a panic), and tests never share directories.
    fn load_from(text: &str) -> Config {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, text).unwrap();
        Config::load(Some(&path))
    }

    #[test]
    fn defaults_parse_with_roster_and_pool() {
        let c = base();
        assert!(!c.messages.pool.is_empty());
        assert_eq!(c.sections.len(), canonical_kind_order().len());
        for kind in canonical_kind_order() {
            assert!(
                matches!(c.sections.get(kind), Some(SectionEntry::CoAuthor(_))),
                "missing built-in section {kind}"
            );
        }
    }

    #[test]
    fn default_config_carries_no_order_and_uses_canonical() {
        let c = base();
        assert!(
            c.order.is_none(),
            "default config must not carry an order list"
        );
        let listed = c.ordered_sections();
        let keys: Vec<&str> = listed.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, canonical_kind_order());
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
    fn user_sections_merge_into_default_roster() {
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
        assert_eq!(
            c.sections.len(),
            canonical_kind_order().len() + 1,
            "adding a section must not remove the built-in roster"
        );
        assert!(matches!(
            c.sections.get("only"),
            Some(SectionEntry::TextLine(s)) if s == "hi"
        ));
        assert!(
            matches!(c.sections.get("os"), Some(SectionEntry::CoAuthor(_))),
            "built-in sections must survive the merge"
        );
    }

    #[test]
    fn user_section_overrides_builtin_same_key() {
        let mut c = base();
        let user = Config {
            commit: CommitConfig::default(),
            messages: MessagesConfig { pool: vec![] },
            sections: BTreeMap::from([(
                "os".to_string(),
                SectionEntry::CoAuthor(CoAuthorConfig {
                    enabled: true,
                    name: Some(FieldValue::Value("My OS".to_string())),
                    email: Some(FieldValue::Value("os@mine.example".to_string())),
                    fields: BTreeMap::new(),
                }),
            )]),
            order: None,
        };
        c.merge(user);
        assert_eq!(c.sections.len(), canonical_kind_order().len());
        match c.sections.get("os") {
            Some(SectionEntry::CoAuthor(cfg)) => {
                assert_eq!(cfg.name, Some(FieldValue::Value("My OS".to_string())));
            }
            _ => panic!("os must still be a CoAuthor"),
        }
        assert!(
            matches!(c.sections.get("kernel"), Some(SectionEntry::CoAuthor(_))),
            "unrelated built-in must stay untouched"
        );
    }

    #[test]
    fn user_sections_without_order_keep_derived_order() {
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
        let mut expected = canonical_kind_order();
        expected.push("bot");
        assert_eq!(
            keys, expected,
            "derived order must render built-ins first, then custom keys"
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
        let mut c = load_from("{}");
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
        let c = load_from("{ this is not json");
        assert!(!c.sections.is_empty());
    }

    #[test]
    fn non_object_config_falls_back_to_defaults() {
        let c = load_from("[1, 2, 3]");
        assert_eq!(c.sections.len(), canonical_kind_order().len());
        assert!(c.order.is_none());
    }

    #[test]
    fn bad_section_field_drops_only_that_section() {
        let c = load_from(
            r#"{
                "sections": {
                    "broken": { "name": "X", "email": "x@x.com", "fields": {"version": 2} },
                    "fine": { "name": "OK", "email": "ok@x.com" }
                },
                "order": ["fine"]
            }"#,
        );
        assert!(
            c.sections.contains_key("fine"),
            "an unrelated valid section must survive one bad entry"
        );
        assert!(
            !c.sections.contains_key("broken"),
            "the malformed section must be dropped, not the whole file"
        );
        assert_eq!(c.order.as_deref(), Some(&["fine".to_string()][..]));
    }

    #[test]
    fn bad_order_dropped_but_sections_kept() {
        let c = load_from(r#"{"sections": {"a": "hi"}, "order": {"not": "an array"}}"#);
        assert!(matches!(c.sections.get("a"), Some(SectionEntry::TextLine(s)) if s == "hi"));
        assert!(
            c.order.is_none(),
            "invalid order must be dropped, sections kept"
        );
    }

    #[test]
    fn null_order_behaves_like_absent() {
        // `"order": null` used to warn spuriously; it must load cleanly and
        // behave exactly like an absent order (derived display order).
        let c = load_from(r#"{"sections": {"a": "hi"}, "order": null}"#);
        assert!(matches!(c.sections.get("a"), Some(SectionEntry::TextLine(s)) if s == "hi"));
        assert!(
            c.order.is_none(),
            "null order must behave like an absent order"
        );
        // The derived display order must be identical to an absent order:
        // built-ins in canonical order, then custom keys alphabetically.
        let c_absent = load_from(r#"{"sections": {"a": "hi"}}"#);
        let keys_null: Vec<String> = c.ordered_sections().into_iter().map(|(k, _)| k).collect();
        let keys_absent: Vec<String> = c_absent
            .ordered_sections()
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(keys_null, keys_absent);
    }

    #[test]
    fn empty_order_is_explicit_empty_list() {
        // Unlike null, an empty array is an explicit "render nothing" list.
        let c = load_from(r#"{"sections": {"a": "hi"}, "order": []}"#);
        assert_eq!(c.order, Some(vec![]));
    }

    #[test]
    fn to_json_roundtrips() {
        let c = base();
        let s = c.to_json();
        let parsed: Config = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed.sections.len(), c.sections.len());
        assert_eq!(
            parsed.order,
            Some(
                canonical_kind_order()
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            ),
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

    fn utf8_neko() -> Config {
        // Shared fixture: Chinese (Traditional) and Japanese config content on
        // the catgirl shi7ku9 theme. The name is written verbatim, never
        // translated.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/utf8-shi7ku9.json");
        Config::load(Some(&path))
    }

    #[test]
    fn utf8_config_loads_chinese_and_japanese() {
        let c = utf8_neko();
        assert_eq!(c.commit.author_name.as_deref(), Some("shi7ku9"));
        assert_eq!(
            c.commit.author_email.as_deref(),
            Some("neko@shi7ku9.example")
        );
        assert_eq!(c.messages.pool, vec!["feat: 我是貓娘，搖著尾巴參上喵！"]);

        assert!(matches!(
            c.sections.get("メモ"),
            Some(SectionEntry::TextLine(s))
                if s == "私は猫娘、お耳ぴこぴこで頑張るにゃ！"
        ));
        match c.sections.get("greeting") {
            Some(SectionEntry::TextField(tf)) => {
                assert_eq!(tf.text, "にゃん！私は猫娘、尻尾ふりふり參上だよ！");
                assert!(tf.fields.is_empty());
            }
            _ => panic!("expected TextField"),
        }
        match c.sections.get("貓娘") {
            Some(SectionEntry::CoAuthor(ca)) => {
                assert_eq!(
                    ca.name,
                    Some(FieldValue::Value("shi7ku9 {rank}".to_string()))
                );
                assert_eq!(
                    ca.email,
                    Some(FieldValue::Value("neko@shi7ku9.example".to_string()))
                );
                assert_eq!(
                    ca.fields.get("rank"),
                    Some(&FieldValue::Value("SSS".to_string()))
                );
            }
            _ => panic!("expected CoAuthor"),
        }
    }

    #[test]
    fn utf8_config_to_json_preserves_chinese_and_japanese() {
        let c = utf8_neko();
        let printed = c.to_json();
        // The printed JSON must carry the same bytes, not mojibake or
        // replacement characters.
        assert!(printed.contains("我是貓娘"));
        assert!(printed.contains("私は猫娘"));
        assert!(printed.contains("私は猫娘、お耳ぴこぴこで頑張るにゃ！"));

        let parsed: Config = serde_json::from_str(&printed).unwrap();
        assert_eq!(parsed.commit.author_name.as_deref(), Some("shi7ku9"));
        assert_eq!(
            parsed.messages.pool,
            vec!["feat: 我是貓娘，搖著尾巴參上喵！"]
        );
        assert!(matches!(
            parsed.sections.get("メモ"),
            Some(SectionEntry::TextLine(s))
                if s == "私は猫娘、お耳ぴこぴこで頑張るにゃ！"
        ));
        match parsed.sections.get("貓娘") {
            Some(SectionEntry::CoAuthor(ca)) => {
                assert_eq!(
                    ca.name,
                    Some(FieldValue::Value("shi7ku9 {rank}".to_string()))
                );
                assert_eq!(
                    ca.email,
                    Some(FieldValue::Value("neko@shi7ku9.example".to_string()))
                );
            }
            _ => panic!("expected CoAuthor"),
        }
    }

    fn unknown(text: &str) -> Vec<String> {
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        super::unknown_keys(&value)
    }

    #[test]
    fn unknown_config_keys_are_reported() {
        let text = r#"{
            "sectionss": {},
            "commit": { "author_nmae": "x" },
            "sections": {
                "os": { "naem": "y" },
                "line": { "text": "hi", "typo": 1 },
                "bot": { "fields": { "v": { "command": "x", "falback": "" } } }
            }
        }"#;
        assert_eq!(
            unknown(text),
            vec![
                "commit.author_nmae",
                "sections.bot.fields.v.falback",
                "sections.line.typo",
                "sections.os.naem",
                "sectionss",
            ]
        );
    }

    #[test]
    fn known_config_keys_are_not_reported() {
        let text = r#"{
            "commit": { "author_name": "A", "author_email": "a@x.com" },
            "messages": { "pool": ["hi"] },
            "sections": {
                "note": "plain text",
                "dynamic": { "text": "Hello {name}", "fields": { "name": "world" } },
                "bot": {
                    "enabled": true,
                    "name": "Bot",
                    "email": "bot@x.com",
                    "fields": { "v": { "command": "x", "fallback": "" } }
                }
            },
            "order": ["note", "dynamic", "bot"]
        }"#;
        assert!(unknown(text).is_empty());
    }

    #[test]
    fn section_shape_selects_allowed_keys() {
        // An object with `text` is a TextField: co-author keys are unknown.
        assert_eq!(
            unknown(r#"{"sections": {"s": { "text": "hi", "name": "x" }}}"#),
            vec!["sections.s.name"]
        );
        // Without `text` it is a co-author: unknown keys are reported there.
        assert_eq!(
            unknown(r#"{"sections": {"s": { "name": "x", "email": "y", "typo": 1 }}}"#),
            vec!["sections.s.typo"]
        );
    }

    #[test]
    fn string_and_array_values_ignored() {
        let text = r#"{
            "messages": { "pool": ["hi", "there"] },
            "sections": { "note": "plain", "other": "text" },
            "order": ["note", "other"]
        }"#;
        assert!(unknown(text).is_empty());
    }

    #[test]
    fn name_and_email_command_objects_are_walked() {
        let text = r#"{
            "sections": {
                "x": {
                    "name": { "command": "printf hi", "typo": 1 },
                    "email": { "command": "printf e", "fallback": "" }
                }
            }
        }"#;
        assert_eq!(unknown(text), vec!["sections.x.name.typo"]);
    }
}
