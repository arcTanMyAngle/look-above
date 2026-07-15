//! `look-above` — a native flight tracker.
//!
//! M0 item 0.5 wires configuration and logging; 0.6 opens the window.

mod config;
mod frame_stats;
mod logging;
mod window;

use anyhow::Result;

use crate::config::{Config, SystemEnv};

fn main() -> Result<()> {
    // Configuration first: it carries the log filter. Failures here predate the subscriber
    // and surface as the process exit error instead.
    let config = Config::load_default(&SystemEnv)?;
    logging::init(&config.log.filter)?;
    log_startup(&config);

    window::run()
}

/// Record what was loaded.
///
/// Credential *values* are never logged (privacy rule 7.1) — only whether they are present.
fn log_startup(config: &Config) {
    if let Some(path) = &config.source_path {
        tracing::info!(path = %path.display(), "loaded configuration");
    } else {
        tracing::info!(
            path = config::DEFAULT_CONFIG_PATH,
            "no configuration file; using defaults"
        );
    }

    tracing::info!(
        opensky_credentials = if config.sources.opensky.is_configured() {
            "configured"
        } else {
            "absent"
        },
        db_path = %config.storage.db_path.display(),
        retention_hours = config.storage.retention_hours,
        "configuration"
    );
}
