//! `tracing` initialization.

use anyhow::{Context, Result, anyhow};
use tracing_subscriber::EnvFilter;

/// Install the global `tracing` subscriber from an `EnvFilter` directive string
/// (e.g. `look_above=debug,warn`).
///
/// The filter arrives from [`crate::config`], so `config.toml` and `LOOK_ABOVE_LOG_FILTER`
/// both reach it through the one precedence chain. `RUST_LOG` is deliberately not
/// consulted: a second variable with its own precedence is a second thing to reason about
/// when logs come out empty.
///
/// Call once, after configuration is loaded. Errors before that point still surface —
/// `main` returns them and they are printed on exit.
pub fn init(filter: &str) -> Result<()> {
    let env_filter =
        EnvFilter::try_new(filter).with_context(|| format!("invalid log filter {filter:?}"))?;

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .try_init()
        .map_err(|err| anyhow!("failed to install the tracing subscriber: {err}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LogConfig;

    // init() installs process-global state, so these exercise the part that can actually
    // fail: parsing the directive string.

    #[test]
    fn the_default_filter_is_a_valid_directive() {
        EnvFilter::try_new(LogConfig::default().filter).expect("the default filter must parse");
    }

    #[test]
    fn a_nonsense_filter_is_rejected() {
        EnvFilter::try_new("look_above=verbose-ish")
            .expect_err("an unknown level is not a valid directive");
    }
}
