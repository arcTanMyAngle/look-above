//! Credential material that cannot reach a log line.
//!
//! Privacy rule 7.1 (docs/04) says credentials never appear in logs, panics, or error
//! messages. [`SecretString`] is that rule as a type rather than as a habit: the `Debug` is
//! redacted and there is deliberately no `Display`, so the only route to the value is
//! [`SecretString::expose`] — one greppable name to audit instead of every format string in
//! the workspace.
//!
//! It lives in `core` because two crates hold credentials: `app` reads them from
//! configuration and `ingest` sends them to `OpenSky`'s token endpoint. `ingest` cannot
//! depend on `app`, so the alternative was a second copy of the redaction — and a rule
//! implemented twice is a rule that only holds in one of them.

use std::fmt;

use serde::Deserialize;

/// A string whose contents never appear in `Debug` output (privacy rule 7.1).
///
/// `Deserialize` is `transparent`, so this drops into a config struct wherever a `String`
/// would go and reads from the same TOML or JSON scalar.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    /// The underlying value.
    ///
    /// Call this only where the credential is actually *used* — the token request — never
    /// when logging, formatting, or building an error message.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether the value is empty or only whitespace.
    ///
    /// A blank credential is treated as an absent one throughout: it is what an untouched
    /// `config.example.toml` and an empty environment variable both look like, and neither
    /// is an attempt to authenticate.
    pub fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }
}

/// Redacted — see the module docs.
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

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_is_redacted() {
        let secret = SecretString::from("hunter2");
        assert_eq!(format!("{secret:?}"), "SecretString(<redacted>)");
        assert!(!format!("{secret:?}").contains("hunter2"));
    }

    /// A secret nested in a larger struct is the realistic leak: the field is redacted, but
    /// only if the derived `Debug` above it actually calls ours.
    #[test]
    fn debug_output_is_redacted_when_nested() {
        #[derive(Debug)]
        struct Holder {
            // Read only by the derived `Debug`, which dead-code analysis does not count.
            #[allow(dead_code)]
            secret: SecretString,
        }
        let rendered = format!(
            "{:?}",
            Holder {
                secret: SecretString::from("hunter2"),
            }
        );
        assert!(!rendered.contains("hunter2"), "leaked: {rendered}");
    }

    #[test]
    fn expose_returns_the_value() {
        assert_eq!(SecretString::from("hunter2").expose(), "hunter2");
    }

    #[test]
    fn blank_values_are_recognized() {
        assert!(SecretString::from("").is_blank());
        assert!(SecretString::from("   ").is_blank());
        assert!(SecretString::from("\t\n").is_blank());
        assert!(!SecretString::from("hunter2").is_blank());
        assert!(!SecretString::from("  hunter2  ").is_blank());
    }

    #[test]
    fn deserializes_from_a_bare_scalar() {
        #[derive(Deserialize)]
        struct Holder {
            secret: SecretString,
        }
        let holder: Holder = serde_json::from_str(r#"{"secret":"hunter2"}"#).expect("parses");
        assert_eq!(holder.secret.expose(), "hunter2");
    }
}
