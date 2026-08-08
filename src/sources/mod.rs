pub mod cpu;
pub mod disk;
pub mod gpu;
pub mod host;
pub mod hostname;
pub mod kernel;
pub mod locale;
pub mod memory;
pub mod os;
pub mod shell;
pub mod terminal;
pub mod uptime;
pub mod user;
pub mod wm;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use sysinfo::System;

use crate::config::{CoAuthorConfig, FieldValue, TextField};
use crate::template::{collapse_whitespace, render};
use crate::util::sh_output;

pub struct CoAuthor {
    pub name: String,
    pub email: String,
}

pub struct SourceContext {
    pub sys: System,
    pub os_info: os_info::Info,
    pub hostname: String,
    pub username: String,
    pub env: HashMap<String, String>,
}

impl SourceContext {
    pub fn new() -> Self {
        let sys = System::new_all();
        let hostname = ::hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_default();
        Self {
            sys,
            os_info: os_info::get(),
            hostname,
            username: whoami::username().unwrap_or_default(),
            env: std::env::vars().collect(),
        }
    }

    pub fn env(&self, key: &str) -> Option<String> {
        self.env.get(key).cloned()
    }
}

pub trait Source {
    fn kind(&self) -> &'static str;
    fn default_name(&self) -> &'static str;
    fn default_email(&self) -> &'static str;
    fn fields(&self, ctx: &SourceContext) -> Option<HashMap<String, String>>;
}

pub fn all_sources() -> Vec<&'static dyn Source> {
    vec![
        &os::Os,
        &kernel::Kernel,
        &host::Host,
        &hostname::Hostname,
        &user::User,
        &shell::Shell,
        &terminal::Terminal,
        &wm::Wm,
        &uptime::Uptime,
        &cpu::Cpu,
        &gpu::Gpu,
        &memory::Memory,
        &disk::Disk,
        &locale::Locale,
    ]
}

/// Runs a user command at most once per `blamefetch` invocation, keyed by the
/// raw command string. Thread-safe: the map lock is held only for the lookup
/// and insertion (microseconds), so distinct commands execute concurrently;
/// identical commands dedupe through the shared `OnceLock`. Empty/trimmed
/// output and any failure resolve to `None`.
#[derive(Default)]
pub struct CommandCache {
    results: Mutex<HashMap<String, Arc<OnceLock<Option<String>>>>>,
}

impl CommandCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn resolve(&self, command: &str) -> Option<String> {
        let slot = self
            .results
            .lock()
            .unwrap()
            .entry(command.to_string())
            .or_insert_with(|| Arc::new(OnceLock::new()))
            .clone();
        slot.get_or_init(|| sh_output(command).filter(|s| !s.is_empty()))
            .clone()
    }
}

/// Resolves one co-author section. `kind` is the section's map key: a known
/// built-in kind pulls the source defaults; any other key is a config-only
/// entry that must supply its own `name`/`email`.
pub fn render_co_author(
    cfg: &CoAuthorConfig,
    kind: &str,
    ctx: &SourceContext,
    cache: &CommandCache,
) -> Option<CoAuthor> {
    if !cfg.enabled {
        return None;
    }

    // Known kind → built-in source; unknown kind is a config-only entry.
    let source = all_sources().into_iter().find(|s| s.kind() == kind);

    // Best-effort source fields; config `fields` override them below. If the
    // source produces nothing (e.g. no lspci), treat as an empty map — the
    // empty name/email guard at the end still fails closed.
    let mut fields: HashMap<String, String> = HashMap::new();
    if let Some(source) = source
        && let Some(src) = source.fields(ctx)
    {
        fields = src;
    }

    // Resolve ALL configured field values first; a command that fails without a
    // fallback kills the whole trailer (fail-closed).
    for (key, fv) in &cfg.fields {
        let value = match fv {
            FieldValue::Value(s) => s.clone(),
            FieldValue::Command { .. } => resolve_command(fv, cache)?,
        };
        fields.insert(key.clone(), value);
    }

    let name = match &cfg.name {
        Some(FieldValue::Value(s)) => render(s, &fields),
        Some(fv @ FieldValue::Command { .. }) => resolve_command(fv, cache)?,
        None => match source {
            Some(s) => render(s.default_name(), &fields),
            None => return None,
        },
    };
    let email = match &cfg.email {
        Some(FieldValue::Value(s)) => render(s, &fields),
        Some(fv @ FieldValue::Command { .. }) => resolve_command(fv, cache)?,
        None => match source {
            Some(s) => render(s.default_email(), &fields),
            None => return None,
        },
    };

    if name.is_empty() || email.is_empty() {
        return None;
    }
    Some(CoAuthor { name, email })
}

/// Resolves a text section: fills `{placeholder}` templates from its `fields`
/// (same command resolution and fail-closed rules as co-author fields).
pub fn render_text(tf: &TextField, cache: &CommandCache) -> Option<String> {
    let mut fields: HashMap<String, String> = HashMap::new();
    for (key, fv) in &tf.fields {
        let value = match fv {
            FieldValue::Value(s) => s.clone(),
            FieldValue::Command { .. } => resolve_command(fv, cache)?,
        };
        fields.insert(key.clone(), value);
    }
    Some(render(&tf.text, &fields))
}

fn resolve_command(fv: &FieldValue, cache: &CommandCache) -> Option<String> {
    match fv {
        FieldValue::Value(_) => None,
        FieldValue::Command { command, fallback } => match cache.resolve(command) {
            Some(out) => Some(collapse_whitespace(&out)),
            None => match fallback {
                Some(f) => Some(collapse_whitespace(f)),
                None => {
                    eprintln!(
                        "blamefetch: warning: command failed and no fallback: {command:?}; skipping section"
                    );
                    None
                }
            },
        },
    }
}

#[cfg(test)]
mod env_sources_test {
    use std::collections::HashMap;

    use sysinfo::System;

    use super::{Source, SourceContext};

    fn ctx(env: &[(&str, &str)]) -> SourceContext {
        SourceContext {
            // Empty system: no process table, so shell detection deterministically
            // finds nothing and the shell tests exercise the `$SHELL` fallback.
            sys: System::new(),
            os_info: os_info::get(),
            hostname: "testhost".to_string(),
            username: "testuser".to_string(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn f(src: &dyn Source, env: &[(&str, &str)]) -> Option<HashMap<String, String>> {
        src.fields(&ctx(env))
    }

    #[test]
    fn hostname_fields() {
        let fields = f(&super::hostname::Hostname, &[]).unwrap();
        assert_eq!(fields.get("hostname").map(String::as_str), Some("testhost"));
    }

    #[test]
    fn user_fields() {
        let fields = f(&super::user::User, &[]).unwrap();
        assert_eq!(fields.get("user").map(String::as_str), Some("testuser"));
    }

    #[test]
    fn terminal_from_env() {
        let fields = f(&super::terminal::Terminal, &[("TERM", "sample")]).unwrap();
        assert_eq!(fields.get("name").map(String::as_str), Some("sample"));
    }

    #[test]
    fn terminal_missing_is_none() {
        assert!(f(&super::terminal::Terminal, &[]).is_none());
    }

    #[test]
    fn locale_from_lang() {
        let fields = f(&super::locale::Locale, &[("LANG", "en_US.UTF-8")]).unwrap();
        assert_eq!(
            fields.get("locale").map(String::as_str),
            Some("en_US.UTF-8")
        );
    }

    #[test]
    fn wm_from_current_desktop() {
        let fields = f(&super::wm::Wm, &[("XDG_CURRENT_DESKTOP", "Hyprland")]).unwrap();
        assert_eq!(fields.get("name").map(String::as_str), Some("Hyprland"));
    }

    #[test]
    fn wm_falls_back_to_desktop_session() {
        let fields = f(&super::wm::Wm, &[("DESKTOP_SESSION", "gnome")]).unwrap();
        assert_eq!(fields.get("name").map(String::as_str), Some("gnome"));
    }

    #[test]
    fn shell_from_env() {
        let fields = f(&super::shell::Shell, &[("SHELL", "/bin/sh")]).unwrap();
        assert_eq!(fields.get("name").map(String::as_str), Some("sh"));
    }

    #[test]
    fn shell_unknown_keeps_name_without_version() {
        let fields = f(&super::shell::Shell, &[("SHELL", "/usr/bin/nushell")]).unwrap();
        assert_eq!(fields.get("name").map(String::as_str), Some("nushell"));
        assert_eq!(fields.get("version").map(String::as_str), Some(""));
    }

    #[test]
    fn shell_empty_is_none() {
        assert!(f(&super::shell::Shell, &[("SHELL", "")]).is_none());
    }

    #[test]
    fn shell_missing_is_none() {
        assert!(f(&super::shell::Shell, &[]).is_none());
    }
}

#[cfg(test)]
mod hardware_sources_test {
    use super::{Source, SourceContext};

    fn ctx() -> SourceContext {
        SourceContext::new()
    }

    fn field(src: &dyn Source, key: &str) -> Option<String> {
        src.fields(&ctx()).and_then(|m| m.get(key).cloned())
    }

    #[test]
    fn os_has_name_and_version() {
        assert!(field(&super::os::Os, "name").is_some_and(|v| !v.is_empty()));
        assert!(field(&super::os::Os, "version").is_some());
    }

    #[test]
    fn kernel_has_release() {
        let release = field(&super::kernel::Kernel, "release");
        if std::env::consts::OS == "linux" {
            assert!(release.is_some_and(|v| !v.is_empty()));
        }
    }

    #[test]
    fn host_optional_returns_consistent_fields() {
        // May be None or Some; if Some, it must carry a non-empty product.
        if let Some(f) = super::host::Host.fields(&ctx()) {
            assert!(f.contains_key("product"));
            assert!(!f["product"].is_empty());
        }
    }

    #[test]
    fn uptime_is_some() {
        assert!(field(&super::uptime::Uptime, "duration").is_some_and(|v| !v.is_empty()));
    }

    #[test]
    fn cpu_fields_present() {
        let fields = super::cpu::Cpu.fields(&ctx());
        assert!(fields.is_some());
        if let Some(f) = fields {
            assert!(f.contains_key("model"));
            assert!(f.contains_key("cores"));
            assert!(f.contains_key("freq"));
        }
    }

    #[test]
    fn memory_fields_consistent() {
        if let Some(f) = super::memory::Memory.fields(&ctx()) {
            assert!(f.contains_key("used"));
            assert!(f.contains_key("total"));
        }
    }

    #[test]
    fn disk_fields_consistent() {
        if let Some(f) = super::disk::Disk.fields(&ctx()) {
            assert!(f.contains_key("used"));
            assert!(f.contains_key("total"));
        }
    }

    #[test]
    fn gpu_optional_returns_consistent_fields() {
        // May be None or Some; if Some, it must carry a non-empty name.
        if let Some(f) = super::gpu::Gpu.fields(&ctx()) {
            assert!(f.contains_key("name"));
            assert!(!f["name"].is_empty());
        }
    }

    #[test]
    fn registry_has_all_spec_kinds_in_order() {
        let kinds: Vec<&str> = super::all_sources().iter().map(|s| s.kind()).collect();
        let expected = [
            "os", "kernel", "host", "hostname", "user", "shell", "terminal", "wm", "uptime", "cpu",
            "gpu", "memory", "disk", "locale",
        ];
        assert_eq!(kinds, expected);
    }
}

#[cfg(test)]
mod co_author_test {
    use std::collections::BTreeMap;

    use crate::config::{CoAuthorConfig, FieldValue, TextField};
    use crate::sources::{CommandCache, SourceContext, render_co_author, render_text};

    fn ctx() -> SourceContext {
        SourceContext::new()
    }

    fn entry(
        name: Option<FieldValue>,
        email: Option<FieldValue>,
        fields: BTreeMap<String, FieldValue>,
    ) -> CoAuthorConfig {
        CoAuthorConfig {
            enabled: true,
            name,
            email,
            fields,
        }
    }

    #[test]
    fn pure_config_static_entry_renders() {
        let cfg = entry(
            Some(FieldValue::Value("Bot".to_string())),
            Some(FieldValue::Value("bot@x.com".to_string())),
            BTreeMap::new(),
        );
        let cache = CommandCache::new();
        let ca = render_co_author(&cfg, "bot", &ctx(), &cache).unwrap();
        assert_eq!(ca.name, "Bot");
        assert_eq!(ca.email, "bot@x.com");
    }

    #[test]
    fn pure_config_missing_name_skips() {
        let cfg = entry(
            None,
            Some(FieldValue::Value("bot@x.com".to_string())),
            BTreeMap::new(),
        );
        let cache = CommandCache::new();
        assert!(render_co_author(&cfg, "bot", &ctx(), &cache).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn command_field_success_substitutes_into_template() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "version".to_string(),
            FieldValue::Command {
                command: "printf 5.0".to_string(),
                fallback: None,
            },
        );
        let cfg = entry(
            Some(FieldValue::Value("Bot {version}".to_string())),
            Some(FieldValue::Value("bot@x.com".to_string())),
            fields,
        );
        let cache = CommandCache::new();
        let ca = render_co_author(&cfg, "bot", &ctx(), &cache).unwrap();
        assert_eq!(ca.name, "Bot 5.0");
    }

    #[cfg(unix)]
    #[test]
    fn command_field_fallback_used_on_failure() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "version".to_string(),
            FieldValue::Command {
                command: "false".to_string(),
                fallback: Some("unknown".to_string()),
            },
        );
        let cfg = entry(
            Some(FieldValue::Value("Bot {version}".to_string())),
            Some(FieldValue::Value("bot@x.com".to_string())),
            fields,
        );
        let cache = CommandCache::new();
        let ca = render_co_author(&cfg, "bot", &ctx(), &cache).unwrap();
        assert_eq!(ca.name, "Bot unknown");
    }

    #[cfg(unix)]
    #[test]
    fn command_field_fails_closed_without_fallback() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "version".to_string(),
            FieldValue::Command {
                command: "false".to_string(),
                fallback: None,
            },
        );
        let cfg = entry(
            Some(FieldValue::Value("Bot {version}".to_string())),
            Some(FieldValue::Value("bot@x.com".to_string())),
            fields,
        );
        let cache = CommandCache::new();
        assert!(render_co_author(&cfg, "bot", &ctx(), &cache).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn empty_output_fails_closed_without_fallback() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "version".to_string(),
            FieldValue::Command {
                command: "true".to_string(),
                fallback: None,
            },
        );
        let cfg = entry(
            Some(FieldValue::Value("Bot {version}".to_string())),
            Some(FieldValue::Value("bot@x.com".to_string())),
            fields,
        );
        let cache = CommandCache::new();
        assert!(render_co_author(&cfg, "bot", &ctx(), &cache).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn failing_command_on_unused_field_still_fails_closed() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "unused".to_string(),
            FieldValue::Command {
                command: "false".to_string(),
                fallback: None,
            },
        );
        let cfg = entry(
            Some(FieldValue::Value("Bot".to_string())),
            Some(FieldValue::Value("bot@x.com".to_string())),
            fields,
        );
        let cache = CommandCache::new();
        assert!(render_co_author(&cfg, "bot", &ctx(), &cache).is_none());
    }

    #[test]
    fn disabled_entry_runs_no_commands() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "version".to_string(),
            FieldValue::Command {
                command: "false".to_string(),
                fallback: None,
            },
        );
        let cfg = CoAuthorConfig {
            enabled: false,
            name: Some(FieldValue::Value("Bot {version}".to_string())),
            email: Some(FieldValue::Value("bot@x.com".to_string())),
            fields,
        };
        let cache = CommandCache::new();
        assert!(render_co_author(&cfg, "bot", &ctx(), &cache).is_none());
        assert!(
            cache.results.lock().unwrap().is_empty(),
            "disabled entries must not resolve commands"
        );
    }

    #[cfg(unix)]
    #[test]
    fn same_command_cached_across_fields() {
        let dir = std::env::temp_dir().join(format!("blamefetch-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("count");
        let cmd = format!("printf x | tee -a {}", marker.display());
        let mut fields = BTreeMap::new();
        fields.insert(
            "a".to_string(),
            FieldValue::Command {
                command: cmd.clone(),
                fallback: None,
            },
        );
        fields.insert(
            "b".to_string(),
            FieldValue::Command {
                command: cmd.clone(),
                fallback: None,
            },
        );
        let cfg = entry(
            Some(FieldValue::Value("{a}{b}".to_string())),
            Some(FieldValue::Value("bot@x.com".to_string())),
            fields,
        );
        let cache = CommandCache::new();
        let ca = render_co_author(&cfg, "bot", &ctx(), &cache).unwrap();
        assert_eq!(ca.name, "xx");
        let count = std::fs::read_to_string(&marker)
            .map(|s| s.len())
            .unwrap_or(0);
        assert_eq!(
            count, 1,
            "identical command must run exactly once per invocation"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn name_as_command_is_used_verbatim() {
        let cfg = entry(
            Some(FieldValue::Command {
                command: "printf 'Bot {version}'".to_string(),
                fallback: None,
            }),
            Some(FieldValue::Value("bot@x.com".to_string())),
            BTreeMap::new(),
        );
        let cache = CommandCache::new();
        let ca = render_co_author(&cfg, "bot", &ctx(), &cache).unwrap();
        assert_eq!(
            ca.name, "Bot {version}",
            "command output is not re-templated"
        );
    }

    #[test]
    fn unknown_kind_without_name_or_email_skips() {
        let cfg = entry(None, None, BTreeMap::new());
        let cache = CommandCache::new();
        assert!(render_co_author(&cfg, "nope", &ctx(), &cache).is_none());
    }

    #[test]
    fn kind_entry_uses_source_defaults() {
        let cfg = entry(None, None, BTreeMap::new());
        let cache = CommandCache::new();
        let ca = render_co_author(&cfg, "os", &ctx(), &cache).unwrap();
        assert!(!ca.name.is_empty());
        assert!(!ca.email.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn render_text_with_command_field_substitutes() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "version".to_string(),
            FieldValue::Command {
                command: "printf 5.0".to_string(),
                fallback: None,
            },
        );
        let tf = TextField {
            text: "Generated by Bot {version}".to_string(),
            fields,
        };
        let cache = CommandCache::new();
        assert_eq!(
            render_text(&tf, &cache).as_deref(),
            Some("Generated by Bot 5.0")
        );
    }

    #[cfg(unix)]
    #[test]
    fn render_text_fails_closed_without_fallback() {
        let mut fields = BTreeMap::new();
        fields.insert(
            "version".to_string(),
            FieldValue::Command {
                command: "false".to_string(),
                fallback: None,
            },
        );
        let tf = TextField {
            text: "v{version}".to_string(),
            fields,
        };
        let cache = CommandCache::new();
        assert!(render_text(&tf, &cache).is_none());
    }

    #[test]
    fn render_text_without_fields_is_verbatim() {
        let tf = TextField {
            text: "hello world".to_string(),
            fields: BTreeMap::new(),
        };
        let cache = CommandCache::new();
        assert_eq!(render_text(&tf, &cache).as_deref(), Some("hello world"));
    }
}
