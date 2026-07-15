//! Application configuration: file, environment, defaults.
//!
//! Values are resolved in precedence order **environment > file > default**: a
//! `LOOK_ABOVE_*` variable beats `config.toml`, which beats the built-in default.
//!
//! Two failure modes are deliberately different (see `plans/DECISION_LOG.md`, 2026-07-15):
//! an **absent** `config.toml` is normal and yields defaults, while one that is **present
//! but unreadable or unparseable** is a hard error. A broken file means the operator meant
//! to configure something; defaulting silently would hide the typo.
//!
//! Credential values never appear in `Debug` output (privacy rule 7.1) — see
//! [`SecretString`].

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// Where [`Config::load_default`] looks, relative to the working directory.
pub const DEFAULT_CONFIG_PATH: &str = "config.toml";

const ENV_LOG_FILTER: &str = "LOOK_ABOVE_LOG_FILTER";
const ENV_OPENSKY_CLIENT_ID: &str = "LOOK_ABOVE_OPENSKY_CLIENT_ID";
const ENV_OPENSKY_CLIENT_SECRET: &str = "LOOK_ABOVE_OPENSKY_CLIENT_SECRET";
const ENV_DB_PATH: &str = "LOOK_ABOVE_DB_PATH";
const ENV_RETENTION_HOURS: &str = "LOOK_ABOVE_RETENTION_HOURS";

/// Our crates at info, the rest of the world at warn.
const DEFAULT_LOG_FILTER: &str = "look_above=info,warn";

const DEFAULT_DB_PATH: &str = "look_above.db";

/// Privacy rule 5.1: position history defaults to 24 h…
pub const RETENTION_HOURS_DEFAULT: u32 = 24;

/// …and may never exceed 7 days.
pub const RETENTION_HOURS_MAX: u32 = 24 * 7;

/// A read-only view of the process environment.
///
/// Injected rather than read directly so tests never touch the real environment:
/// `std::env::set_var` is `unsafe` in edition 2024, and the environment is process-global
/// state that parallel tests would race on.
pub trait EnvSource {
    /// The value of `key`, or `None` when it is unset.
    fn var(&self, key: &str) -> Option<String>;
}

/// The real process environment.
#[derive(Debug, Clone, Copy)]
pub struct SystemEnv;

impl EnvSource for SystemEnv {
    fn var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// Credential material, with a redacted `Debug` so it cannot reach a log line or a panic
/// message (privacy rule 7.1).
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    /// The underlying value.
    ///
    /// Call this only where the credential is actually used — the `OpenSky` token request —
    /// never when logging or formatting.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// The whole of the application's configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub log: LogConfig,
    pub sources: SourcesConfig,
    pub storage: StorageConfig,

    /// The file this was read from, or `None` when no file existed and defaults were used.
    /// Not a config key — [`Config::load`] fills it in.
    #[serde(skip)]
    pub source_path: Option<PathBuf>,
}

/// Logging.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LogConfig {
    /// A `tracing-subscriber` `EnvFilter` directive string, e.g. `look_above=debug,warn`.
    pub filter: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            filter: DEFAULT_LOG_FILTER.to_owned(),
        }
    }
}

/// Live data sources.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SourcesConfig {
    pub opensky: OpenSkyConfig,
}

/// `OpenSky` `OAuth2` client-credentials.
///
/// Both are absent until the owner creates a free API client (M1 item 1.3); the no-key
/// community fallbacks need no credentials at all.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OpenSkyConfig {
    pub client_id: Option<SecretString>,
    pub client_secret: Option<SecretString>,
}

impl OpenSkyConfig {
    /// Whether both halves of the credential are present.
    ///
    /// [`Config::load`] rejects half a pair, so this answers "can we authenticate?".
    pub fn is_configured(&self) -> bool {
        self.client_id.is_some() && self.client_secret.is_some()
    }
}

/// Local persistence.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    /// The `SQLite` database file, relative to the working directory unless absolute.
    pub db_path: PathBuf,

    /// How long position history is kept, in hours.
    ///
    /// Privacy rule 5.1: default 24 h, hard maximum 7 days, configurable downward only.
    pub retention_hours: u32,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from(DEFAULT_DB_PATH),
            retention_hours: RETENTION_HOURS_DEFAULT,
        }
    }
}

impl Config {
    /// Load from [`DEFAULT_CONFIG_PATH`] in the working directory, with environment
    /// overrides applied on top.
    pub fn load_default(env: &impl EnvSource) -> Result<Self> {
        Self::load(Path::new(DEFAULT_CONFIG_PATH), env)
    }

    /// Load from `path`, then apply `LOOK_ABOVE_*` overrides from `env`.
    ///
    /// A missing `path` is not an error: it yields defaults, plus any environment
    /// overrides. Every other read failure, and any file that does not parse, is an error.
    pub fn load(path: &Path, env: &impl EnvSource) -> Result<Self> {
        let mut config = match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut parsed: Self = toml::from_str(&text)
                    .with_context(|| format!("{} is not valid configuration", path.display()))?;
                parsed.source_path = Some(path.to_path_buf());
                parsed
            }
            Err(err) if err.kind() == ErrorKind::NotFound => Self::default(),
            Err(err) => {
                return Err(err).with_context(|| format!("cannot read {}", path.display()));
            }
        };

        config.apply_env(env)?;
        config.normalize();
        config.validate()?;
        Ok(config)
    }

    /// Overlay `LOOK_ABOVE_*` variables. Present-but-unparseable values are errors, for the
    /// same reason a broken file is.
    fn apply_env(&mut self, env: &impl EnvSource) -> Result<()> {
        if let Some(value) = env.var(ENV_LOG_FILTER) {
            self.log.filter = value;
        }
        if let Some(value) = env.var(ENV_OPENSKY_CLIENT_ID) {
            self.sources.opensky.client_id = Some(SecretString::from(value));
        }
        if let Some(value) = env.var(ENV_OPENSKY_CLIENT_SECRET) {
            self.sources.opensky.client_secret = Some(SecretString::from(value));
        }
        if let Some(value) = env.var(ENV_DB_PATH) {
            self.storage.db_path = PathBuf::from(value);
        }
        if let Some(value) = env.var(ENV_RETENTION_HOURS) {
            self.storage.retention_hours = value.trim().parse().with_context(|| {
                format!("{ENV_RETENTION_HOURS} must be a whole number of hours, got {value:?}")
            })?;
        }
        Ok(())
    }

    /// A blank credential means "absent", so that `config.example.toml` behaves like no
    /// file when copied verbatim, and an empty `LOOK_ABOVE_OPENSKY_CLIENT_ID=` does not
    /// masquerade as a credential.
    fn normalize(&mut self) {
        let opensky = &mut self.sources.opensky;
        opensky.client_id = opensky
            .client_id
            .take()
            .filter(|secret| !secret.expose().trim().is_empty());
        opensky.client_secret = opensky
            .client_secret
            .take()
            .filter(|secret| !secret.expose().trim().is_empty());
    }

    fn validate(&self) -> Result<()> {
        if self.log.filter.trim().is_empty() {
            bail!(
                "log.filter is empty; remove the key to take the default ({DEFAULT_LOG_FILTER:?})"
            );
        }
        if self.storage.db_path.as_os_str().is_empty() {
            bail!(
                "storage.db_path is empty; remove the key to take the default ({DEFAULT_DB_PATH:?})"
            );
        }
        if self.storage.retention_hours > RETENTION_HOURS_MAX {
            bail!(
                "storage.retention_hours is {} but may not exceed {RETENTION_HOURS_MAX} \
                 (7 days — privacy rule 5.1); history is configurable downward only",
                self.storage.retention_hours
            );
        }
        // Half a pair cannot authenticate, and reads as a typo rather than an intent to run
        // without credentials.
        if self.sources.opensky.client_id.is_some() != self.sources.opensky.client_secret.is_some()
        {
            bail!("sources.opensky needs both client_id and client_secret, or neither");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    impl EnvSource for BTreeMap<String, String> {
        fn var(&self, key: &str) -> Option<String> {
            self.get(key).cloned()
        }
    }

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn empty_env() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    /// A self-cleaning temp directory — enough for these tests without a `tempfile`
    /// dev-dependency.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "look-above-cfg-{}-{label}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("create temp dir in test");
            Self { path }
        }

        /// The path `config.toml` would occupy — deliberately not created.
        fn config_path(&self) -> PathBuf {
            self.path.join("config.toml")
        }

        fn write_config(&self, contents: &str) -> PathBuf {
            let path = self.config_path();
            std::fs::write(&path, contents).expect("write config in test");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn exposed(secret: Option<&SecretString>) -> Option<&str> {
        secret.map(SecretString::expose)
    }

    // --- Acceptance §M0: "missing file yields defaults, not error" -----------------------

    #[test]
    fn missing_file_yields_defaults_not_an_error() {
        let dir = TempDir::new("missing");
        let config = Config::load(&dir.config_path(), &empty_env())
            .expect("a missing config.toml is not an error");

        assert_eq!(config, Config::default());
        assert_eq!(
            config.source_path, None,
            "nothing was read, so no source path"
        );
    }

    #[test]
    fn empty_file_is_defaults() {
        // Distinct from malformed: an empty file is valid TOML with every key absent.
        let dir = TempDir::new("empty");
        let path = dir.write_config("");
        let config = Config::load(&path, &empty_env()).expect("an empty config.toml is valid");

        assert_eq!(
            Config {
                source_path: None,
                ..config
            },
            Config::default()
        );
    }

    // --- Acceptance §M0: "config loads from config.toml + env override" ------------------

    #[test]
    fn file_values_are_read() {
        let dir = TempDir::new("file");
        let path = dir.write_config(
            r#"
            [log]
            filter = "look_above=debug"

            [sources.opensky]
            client_id = "id-from-file"
            client_secret = "secret-from-file"

            [storage]
            db_path = "custom.db"
            retention_hours = 6
            "#,
        );
        let config = Config::load(&path, &empty_env()).expect("valid config parses");

        assert_eq!(config.log.filter, "look_above=debug");
        assert_eq!(
            exposed(config.sources.opensky.client_id.as_ref()),
            Some("id-from-file")
        );
        assert_eq!(
            exposed(config.sources.opensky.client_secret.as_ref()),
            Some("secret-from-file")
        );
        assert_eq!(config.storage.db_path, PathBuf::from("custom.db"));
        assert_eq!(config.storage.retention_hours, 6);
        assert_eq!(config.source_path.as_deref(), Some(path.as_path()));
    }

    #[test]
    fn env_overrides_file() {
        let dir = TempDir::new("env-over-file");
        let path = dir.write_config(
            r#"
            [log]
            filter = "look_above=debug"

            [sources.opensky]
            client_id = "id-from-file"
            client_secret = "secret-from-file"

            [storage]
            db_path = "from-file.db"
            retention_hours = 6
            "#,
        );
        let config = Config::load(
            &path,
            &env(&[
                ("LOOK_ABOVE_LOG_FILTER", "look_above=trace"),
                ("LOOK_ABOVE_OPENSKY_CLIENT_ID", "id-from-env"),
                ("LOOK_ABOVE_OPENSKY_CLIENT_SECRET", "secret-from-env"),
                ("LOOK_ABOVE_DB_PATH", "from-env.db"),
                ("LOOK_ABOVE_RETENTION_HOURS", "12"),
            ]),
        )
        .expect("valid config parses");

        assert_eq!(config.log.filter, "look_above=trace");
        assert_eq!(
            exposed(config.sources.opensky.client_id.as_ref()),
            Some("id-from-env")
        );
        assert_eq!(
            exposed(config.sources.opensky.client_secret.as_ref()),
            Some("secret-from-env")
        );
        assert_eq!(config.storage.db_path, PathBuf::from("from-env.db"));
        assert_eq!(config.storage.retention_hours, 12);
    }

    #[test]
    fn env_applies_without_a_file() {
        // Privacy rule 7.1 names environment variables as a home for credentials, so they
        // have to work with no config.toml present at all.
        let dir = TempDir::new("env-only");
        let config = Config::load(
            &dir.config_path(),
            &env(&[
                ("LOOK_ABOVE_OPENSKY_CLIENT_ID", "id-from-env"),
                ("LOOK_ABOVE_OPENSKY_CLIENT_SECRET", "secret-from-env"),
                ("LOOK_ABOVE_RETENTION_HOURS", "1"),
            ]),
        )
        .expect("env-only configuration is valid");

        assert!(config.sources.opensky.is_configured());
        assert_eq!(config.storage.retention_hours, 1);
        assert_eq!(
            config.log.filter, DEFAULT_LOG_FILTER,
            "keys no variable touched keep their defaults"
        );
    }

    #[test]
    fn env_overrides_only_the_keys_it_names() {
        let dir = TempDir::new("partial-env");
        let path = dir.write_config("[storage]\nretention_hours = 6\ndb_path = \"kept.db\"\n");
        let config = Config::load(&path, &env(&[("LOOK_ABOVE_RETENTION_HOURS", "3")]))
            .expect("valid config parses");

        assert_eq!(config.storage.retention_hours, 3, "overridden");
        assert_eq!(
            config.storage.db_path,
            PathBuf::from("kept.db"),
            "left to the file"
        );
    }

    // --- Acceptance §M0: "repo contains config.example.toml" -----------------------------

    #[test]
    fn example_file_is_equivalent_to_no_file() {
        // include_str! is compile-time: this test failing to build means the example file
        // went missing.
        const EXAMPLE: &str = include_str!("../../../config.example.toml");

        let dir = TempDir::new("example");
        let path = dir.write_config(EXAMPLE);
        let config = Config::load(&path, &empty_env()).expect("config.example.toml parses");

        assert_eq!(
            Config {
                source_path: None,
                ..config
            },
            Config::default(),
            "copying config.example.toml verbatim must behave exactly like having no file"
        );
    }

    // --- A present-but-broken file is NOT the missing-file case ---------------------------

    #[test]
    fn malformed_file_is_an_error_not_defaults() {
        let dir = TempDir::new("malformed");
        let path = dir.write_config("[log\nfilter = ");
        let err = Config::load(&path, &empty_env())
            .expect_err("a present-but-broken config.toml must not silently default");

        assert!(
            err.to_string().contains("not valid configuration"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn unknown_key_is_an_error() {
        // The point of deny_unknown_fields: a typo'd key must not read as "absent", which
        // is exactly how a credential goes silently missing.
        let dir = TempDir::new("unknown-key");
        let path =
            dir.write_config("[sources.opensky]\nclientid = \"typo\"\nclient_secret = \"s\"\n");
        let err = Config::load(&path, &empty_env()).expect_err("unknown keys are rejected");

        assert!(
            err.to_string().contains("not valid configuration"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn wrong_type_is_an_error() {
        let dir = TempDir::new("wrong-type");
        let path = dir.write_config("[storage]\nretention_hours = \"lots\"\n");

        Config::load(&path, &empty_env()).expect_err("retention_hours must be an integer");
    }

    #[test]
    fn unreadable_file_is_an_error() {
        // A directory standing where config.toml should be: present, unreadable — and so
        // not the "absent" case.
        let dir = TempDir::new("unreadable");
        let path = dir.config_path();
        std::fs::create_dir_all(&path).expect("create dir in test");

        Config::load(&path, &empty_env()).expect_err("an unreadable config.toml is not defaults");
    }

    #[test]
    fn unparseable_env_override_is_an_error() {
        let dir = TempDir::new("bad-env");
        let err = Config::load(
            &dir.config_path(),
            &env(&[("LOOK_ABOVE_RETENTION_HOURS", "soon")]),
        )
        .expect_err("a non-numeric retention override is an error");

        assert!(
            err.to_string().contains(ENV_RETENTION_HOURS),
            "the message must name the variable at fault: {err}"
        );
    }

    // --- Privacy rules -------------------------------------------------------------------

    #[test]
    fn retention_above_the_privacy_cap_is_rejected() {
        let dir = TempDir::new("retention-cap");
        let path = dir.write_config(&format!(
            "[storage]\nretention_hours = {}\n",
            RETENTION_HOURS_MAX + 1
        ));
        let err = Config::load(&path, &empty_env())
            .expect_err("privacy rule 5.1 caps retention at 7 days");

        assert!(
            err.to_string().contains("privacy rule 5.1"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn retention_at_the_cap_is_allowed() {
        let dir = TempDir::new("retention-max");
        let path = dir.write_config(&format!(
            "[storage]\nretention_hours = {RETENTION_HOURS_MAX}\n"
        ));
        let config = Config::load(&path, &empty_env()).expect("the cap itself is a legal value");

        assert_eq!(config.storage.retention_hours, RETENTION_HOURS_MAX);
    }

    #[test]
    fn retention_is_configurable_downward() {
        let dir = TempDir::new("retention-down");
        let path = dir.write_config("[storage]\nretention_hours = 0\n");
        let config =
            Config::load(&path, &empty_env()).expect("keeping no history is the private extreme");

        assert_eq!(config.storage.retention_hours, 0);
    }

    #[test]
    fn secrets_are_redacted_in_debug_output() {
        // Privacy rule 7.1: credentials must never reach a log, and Debug is how they would.
        let secret = SecretString::from("hunter2".to_owned());
        assert_eq!(format!("{secret:?}"), "SecretString(<redacted>)");

        let config = Config {
            sources: SourcesConfig {
                opensky: OpenSkyConfig {
                    client_id: Some(SecretString::from("id-abc".to_owned())),
                    client_secret: Some(SecretString::from("hunter2".to_owned())),
                },
            },
            ..Config::default()
        };
        let rendered = format!("{config:?}");

        assert!(
            !rendered.contains("hunter2"),
            "client secret leaked into Debug: {rendered}"
        );
        assert!(
            !rendered.contains("id-abc"),
            "client id leaked into Debug: {rendered}"
        );
    }

    // --- Credential shape ----------------------------------------------------------------

    #[test]
    fn blank_credentials_read_as_absent() {
        let dir = TempDir::new("blank-creds");
        let path =
            dir.write_config("[sources.opensky]\nclient_id = \"\"\nclient_secret = \"   \"\n");
        let config = Config::load(&path, &empty_env()).expect("blank credentials are not an error");

        assert_eq!(config.sources.opensky.client_id, None);
        assert_eq!(config.sources.opensky.client_secret, None);
        assert!(!config.sources.opensky.is_configured());
    }

    #[test]
    fn blank_env_credential_does_not_masquerade_as_one() {
        let dir = TempDir::new("blank-env-cred");
        let config = Config::load(
            &dir.config_path(),
            &env(&[
                ("LOOK_ABOVE_OPENSKY_CLIENT_ID", ""),
                ("LOOK_ABOVE_OPENSKY_CLIENT_SECRET", ""),
            ]),
        )
        .expect("an empty variable reads as unset, not as half a credential");

        assert!(!config.sources.opensky.is_configured());
    }

    #[test]
    fn half_a_credential_is_an_error() {
        let dir = TempDir::new("half-cred");
        let path = dir.write_config("[sources.opensky]\nclient_id = \"only-the-id\"\n");
        let err = Config::load(&path, &empty_env())
            .expect_err("half a credential pair cannot authenticate");

        assert!(
            err.to_string().contains("both client_id and client_secret"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn env_can_complete_a_credential_the_file_half_declares() {
        // The documented split: id in config.toml, secret from the environment.
        let dir = TempDir::new("split-cred");
        let path = dir.write_config("[sources.opensky]\nclient_id = \"id-from-file\"\n");
        let config = Config::load(
            &path,
            &env(&[("LOOK_ABOVE_OPENSKY_CLIENT_SECRET", "secret-from-env")]),
        )
        .expect("file id + env secret is a complete credential");

        assert!(config.sources.opensky.is_configured());
        assert_eq!(
            exposed(config.sources.opensky.client_id.as_ref()),
            Some("id-from-file")
        );
    }

    // --- Filter / path sanity -------------------------------------------------------------

    #[test]
    fn empty_log_filter_is_an_error() {
        let dir = TempDir::new("empty-filter");
        let path = dir.write_config("[log]\nfilter = \"\"\n");

        Config::load(&path, &empty_env()).expect_err("an empty filter would silence the app");
    }

    #[test]
    fn empty_db_path_is_an_error() {
        let dir = TempDir::new("empty-db-path");
        let path = dir.write_config("[storage]\ndb_path = \"\"\n");

        Config::load(&path, &empty_env()).expect_err("an empty db_path names no file");
    }
}
