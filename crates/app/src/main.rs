//! `look-above` — a native flight tracker.
//!
//! M0 item 0.5 wires configuration and logging; 0.6 opens the window. M1 item 1.12 adds
//! `--headless`, the mode the gate run (item 1.13) drives the pipeline through with no GPU
//! and no display attached.

mod config;
mod double_buffer;
mod enrichment;
mod frame_stats;
mod headless;
mod logging;
mod pipeline;
mod simulation;
mod window;

use anyhow::{Result, bail};

use crate::config::{Config, SystemEnv};

/// Which loop `main` hands off to, decided by [`parse_args`].
#[derive(Debug)]
enum Mode {
    /// The default: open a window and render.
    Window,
    /// `--headless`: no window, no GPU — just the ingest pipeline logging per-cycle counts.
    Headless,
}

/// Reads `--headless` from the real process arguments (`argv[0]` skipped).
///
/// An unrecognized argument is a hard error rather than a silent ignore, the same call
/// `config` makes for an unknown key: a typo'd flag should not quietly run the default mode.
fn parse_args() -> Result<Mode> {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from(args: impl Iterator<Item = String>) -> Result<Mode> {
    let mut mode = Mode::Window;
    for arg in args {
        match arg.as_str() {
            "--headless" => mode = Mode::Headless,
            other => bail!("unrecognized argument: {other} (only --headless is accepted)"),
        }
    }
    Ok(mode)
}

fn main() -> Result<()> {
    // Configuration first: it carries the log filter. Failures here predate the subscriber
    // and surface as the process exit error instead.
    let config = Config::load_default(&SystemEnv)?;
    logging::init(&config.log.filter)?;
    log_startup(&config);

    match parse_args()? {
        Mode::Window => window::run(&config),
        Mode::Headless => headless::run(&config),
    }
}

/// Record what was loaded.
///
/// Credential *values* are never logged (privacy rule 7.1) — only whether they are present,
/// and which file they came from. That last part earns its line: with three possible
/// sources for one credential, "absent" is a question ("but I put the file *there*") unless
/// the log says where we looked.
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
        opensky_credentials_from = match &config.credentials_path {
            Some(path) => path.display().to_string(),
            None if config.sources.opensky.is_configured() =>
                "config.toml or environment".to_owned(),
            None => format!("nothing (looked for {})", config::DEFAULT_CREDENTIALS_PATH),
        },
        db_path = %config.storage.db_path.display(),
        retention_hours = config.storage.retention_hours,
        "configuration"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn no_arguments_is_window_mode() {
        assert!(matches!(
            parse_args_from(args(&[]).into_iter()).expect("no arguments parses"),
            Mode::Window
        ));
    }

    #[test]
    fn the_headless_flag_selects_headless_mode() {
        assert!(matches!(
            parse_args_from(args(&["--headless"]).into_iter()).expect("--headless parses"),
            Mode::Headless
        ));
    }

    #[test]
    fn an_unrecognized_argument_is_a_hard_error() {
        let err = parse_args_from(args(&["--bogus"]).into_iter())
            .expect_err("an unknown flag is rejected");
        assert!(
            err.to_string().contains("--bogus"),
            "the message must name the argument at fault: {err}"
        );
    }
}
