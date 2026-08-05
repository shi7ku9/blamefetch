use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub commit: CommitConfig,
    #[serde(default)]
    pub messages: MessagesConfig,
    #[serde(default)]
    pub co_authors: Vec<CoAuthorConfig>,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CoAuthorConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<FieldValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<FieldValue>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, FieldValue>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub blank_line_before: bool,
}

fn default_true() -> bool {
    true
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Config {
    pub fn load(explicit: Option<&Path>) -> Config {
        let mut cfg: Config = toml::from_str(include_str!("default-config.toml"))
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
                Ok(text) => match toml::from_str::<Config>(&text) {
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

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("config must serialize")
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
        if !user.co_authors.is_empty() {
            self.co_authors = user.co_authors;
        }
    }
}

fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("blamefetch").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde::Deserialize;

    use super::{CoAuthorConfig, CommitConfig, Config, FieldValue, MessagesConfig};

    fn base() -> Config {
        Config::load(None)
    }

    #[test]
    fn defaults_parse_with_roster_and_pool() {
        let c = base();
        assert!(!c.messages.pool.is_empty());
        assert!(
            !c.co_authors
                .iter()
                .any(|ca| ca.kind.as_deref() == Some("sample")),
            "no sample entry in defaults"
        );
        // machine sources present and in spec order
        let kinds: Vec<&str> = c
            .co_authors
            .iter()
            .filter_map(|ca| ca.kind.as_deref())
            .collect();
        let expected = [
            "os", "kernel", "host", "hostname", "user", "shell", "terminal", "wm", "uptime", "cpu",
            "gpu", "memory", "disk", "locale",
        ];
        assert_eq!(kinds, expected);
    }

    #[test]
    fn user_pool_replaces_default() {
        let mut c = base();
        let user = Config {
            commit: CommitConfig::default(),
            messages: MessagesConfig {
                pool: vec!["mine".to_string()],
            },
            co_authors: vec![],
        };
        c.merge(user);
        assert_eq!(c.messages.pool, vec!["mine".to_string()]);
        assert!(
            !c.co_authors.is_empty(),
            "co_authors must stay from defaults"
        );
    }

    #[test]
    fn user_co_authors_replace_roster() {
        let mut c = base();
        let user = Config {
            commit: CommitConfig::default(),
            messages: MessagesConfig { pool: vec![] },
            co_authors: vec![CoAuthorConfig {
                kind: Some("sample".to_string()),
                enabled: true,
                name: Some(FieldValue::Value("Sample {version}".to_string())),
                email: Some(FieldValue::Value("noreply@example.com".to_string())),
                fields: BTreeMap::new(),
                blank_line_before: true,
            }],
        };
        c.merge(user);
        assert_eq!(c.co_authors.len(), 1);
    }

    #[test]
    fn commit_fields_merge_fieldwise() {
        // Load from an empty file so the developer's real config
        // (~/.config/blamefetch/config.toml) cannot leak into the base.
        // Distinct from the dir used by bad_config_falls_back_to_defaults: the
        // two tests run in parallel threads and would delete each other's files.
        let dir = std::env::temp_dir().join(format!("blamefetch-cfg-merge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.toml");
        std::fs::write(&path, "").unwrap();
        let mut c = Config::load(Some(&path));
        std::fs::remove_dir_all(&dir).unwrap();
        let user = Config {
            commit: CommitConfig {
                author_name: Some("A".to_string()),
                author_email: None,
            },
            messages: MessagesConfig { pool: vec![] },
            co_authors: vec![],
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
        let path = dir.join("bad.toml");
        std::fs::write(&path, "this is not [ toml").unwrap();
        let c = Config::load(Some(&path));
        assert!(!c.co_authors.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn to_toml_roundtrips() {
        let c = base();
        let s = c.to_toml();
        let parsed: Config = toml::from_str(&s).unwrap();
        assert_eq!(parsed.co_authors.len(), c.co_authors.len());
    }

    #[test]
    fn field_value_untagged_parses() {
        #[derive(Deserialize)]
        struct Wrap {
            v: FieldValue,
        }
        let s: Wrap = toml::from_str("v = \"literal\"").unwrap();
        assert_eq!(s.v, FieldValue::Value("literal".to_string()));

        let c: Wrap =
            toml::from_str(r#"v = { command = "sample --version | cut -d' ' -f1" }"#).unwrap();
        assert_eq!(
            c.v,
            FieldValue::Command {
                command: "sample --version | cut -d' ' -f1".to_string(),
                fallback: None,
            }
        );

        let f: Wrap = toml::from_str(r#"v = { command = "x", fallback = "y" }"#).unwrap();
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
            co_authors: vec![CoAuthorConfig {
                kind: None,
                enabled: true,
                name: Some(FieldValue::Value("Sample {version}".to_string())),
                email: Some(FieldValue::Value("noreply@example.com".to_string())),
                fields,
                blank_line_before: true,
            }],
        };
        let s = cfg.to_toml();
        // The `toml` crate emits `[co_authors.fields]` when the fields map holds
        // scalar members, but collapses a map that holds only nested tables into
        // dotted-key form (`[co_authors.fields.version]`). Accept either shape.
        assert!(
            s.contains("[co_authors.fields"),
            "toml must emit the nested fields table under co_authors:\n{s}"
        );
        let parsed: Config = toml::from_str(&s).unwrap();
        assert_eq!(parsed.co_authors.len(), 1);
        let ca = &parsed.co_authors[0];
        assert!(ca.kind.is_none());
        assert!(ca.blank_line_before);
        match &ca.fields["version"] {
            FieldValue::Command { command, fallback } => {
                assert_eq!(command, "sample --version | cut -d' ' -f1");
                assert_eq!(fallback.as_deref(), Some("unknown"));
            }
            _ => panic!("expected Command"),
        }
    }

    #[test]
    fn kind_is_optional_and_blank_line_before_defaults_false() {
        let c: Config =
            toml::from_str("[[co_authors]]\nname = \"Only\"\nemail = \"only@x.com\"\n").unwrap();
        assert_eq!(c.co_authors.len(), 1);
        assert!(c.co_authors[0].kind.is_none());
        assert!(!c.co_authors[0].blank_line_before);
        assert!(c.co_authors[0].enabled);
    }
}
