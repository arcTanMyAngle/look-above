//! The exhaustive list of hosts this crate may contact, and the gate that enforces it.
//!
//! Privacy rule 1.1 makes the authorized-aviation-sources skill the sole authority on where
//! data may come from; this module is that list expressed in code. docs/10 §privacy asks for
//! "a single const list of permitted hosts" plus a test walking adapter base URLs — see
//! [`AUTHORIZED_HOSTS`] and the tests below.
//!
//! The list is not decoration: [`HttpClient::get`](crate::http::HttpClient::get) checks every
//! URL against it before a request exists, and the client's redirect policy checks every hop.
//! A test that walks the base URLs adapters *declare* can only see the URLs adapters declare;
//! the gate sees the URL that would actually go on the wire.
//!
//! **Scope.** These are the hosts reachable through [`crate::http`] at runtime. The static
//! bulk downloads the skill also authorizes — `OurAirports`, the FAA registry, openflights,
//! Natural Earth — are fetched by import tooling at setup time, not by this crate, and are
//! deliberately absent: `raw.githubusercontent.com` is a host that serves anyone's repository,
//! and widening the live-polling gate to cover a build step it never uses buys nothing. When
//! that tooling lands it extends this list on purpose, with a decision-log entry.

use look_above_core::error::SourceError;
use reqwest::Url;

/// Every host `ingest` may contact, from the authorized-aviation-sources skill.
///
/// Adding an entry requires owner approval and a `plans/DECISION_LOG.md` entry confirming the
/// source's terms permit free programmatic use (privacy rule 1.1). Removing the gate around it
/// is not an option a bug should be able to reach.
pub const AUTHORIZED_HOSTS: &[&str] = &[
    // Live positions — primary. The API and the OAuth2 token endpoint are separate hosts.
    "opensky-network.org",
    "auth.opensky-network.org",
    // Live positions — fallbacks, in failover order.
    "api.airplanes.live",
    "api.adsb.lol",
    // Enrichment — aircraft metadata and routes. Selection path only, and only for targets
    // whose `anonymous` flag is false (privacy rule 2.2 — that gate lives at the call site).
    "api.adsbdb.com",
    // Enrichment — METAR/TAF, official NOAA.
    "aviationweather.gov",
];

/// Whether `host` is on the allowlist.
///
/// Matching is exact and case-insensitive (hosts are case-insensitive per RFC 3986). It is
/// deliberately not a suffix match: `ends_with("opensky-network.org")` would also welcome
/// `evil-opensky-network.org`, which is the classic way an allowlist becomes a formality.
/// Subdomains that are genuinely needed are listed in full, as `auth.` is above.
pub fn is_authorized_host(host: &str) -> bool {
    AUTHORIZED_HOSTS
        .iter()
        .any(|authorized| authorized.eq_ignore_ascii_case(host))
}

/// Which URLs a given [`HttpClient`](crate::http::HttpClient) may request.
///
/// A policy rather than a bare function so the test variant can exist without production code
/// having a branch that disables the gate. There is no feature flag here on purpose: a
/// `testing` feature could be unified into the app's dependency graph by an unrelated crate
/// and silently switch the gate off in a shipped binary, whereas `#[cfg(test)]` cannot escape
/// this crate's own test build.
#[derive(Debug, Clone, Copy)]
pub(crate) enum HostPolicy {
    /// [`AUTHORIZED_HOSTS`], over HTTPS. The only policy reachable from a release build.
    Authorized,
    /// Also permits loopback, so tests can point the real client at a local mock server.
    #[cfg(test)]
    AuthorizedOrLoopback,
}

impl HostPolicy {
    /// Fails closed: anything not positively recognized is refused.
    pub(crate) fn check(self, url: &Url) -> Result<(), SourceError> {
        if self.permits(url) {
            return Ok(());
        }
        Err(SourceError::Refused {
            reason: format!(
                "{} is not an authorized origin (privacy rule 1.1)",
                origin(url)
            ),
        })
    }

    pub(crate) fn permits(self, url: &Url) -> bool {
        match self {
            Self::Authorized => {
                url.scheme() == "https" && url.host_str().is_some_and(is_authorized_host)
            }
            #[cfg(test)]
            Self::AuthorizedOrLoopback => is_loopback(url) || Self::Authorized.permits(url),
        }
    }
}

/// Renders scheme + host for an error message — never the path or query.
///
/// Privacy rule 7.1: a source that takes a token as a query parameter would otherwise put one
/// into every refusal we log. The origin is the whole of what was refused, and it is enough to
/// find the offending call.
fn origin(url: &Url) -> String {
    match url.host_str() {
        Some(host) => format!("{}://{host}", url.scheme()),
        None => format!("a {} URL with no host", url.scheme()),
    }
}

/// Loopback only — `localhost`, `127.0.0.0/8`, `::1`.
///
/// Host strings carry IPv6 literals in brackets, hence the trim. Matched on the address
/// rather than the text so that `127.0.0.2`, which wiremock may hand out, is covered too.
#[cfg(test)]
fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).expect("test URL parses")
    }

    #[test]
    fn the_list_is_the_skill_list() {
        // Pinned literally, so a quiet edit to the const has to be a deliberate edit to a
        // test that names the source of truth. Compare against
        // .claude/skills/authorized-aviation-sources/SKILL.md.
        assert_eq!(
            AUTHORIZED_HOSTS,
            [
                "opensky-network.org",
                "auth.opensky-network.org",
                "api.airplanes.live",
                "api.adsb.lol",
                "api.adsbdb.com",
                "aviationweather.gov",
            ]
        );
    }

    #[test]
    fn authorized_hosts_match_exactly_and_ignore_case() {
        assert!(is_authorized_host("opensky-network.org"));
        assert!(is_authorized_host("OpenSky-Network.ORG"));
        assert!(is_authorized_host("auth.opensky-network.org"));
    }

    #[test]
    fn lookalike_hosts_are_not_authorized() {
        // Each of these passes a `contains`/`ends_with`/`starts_with` allowlist. That is the
        // point of the test: they must all fail this one.
        for host in [
            "evil-opensky-network.org",
            "opensky-network.org.evil.com",
            "api.adsb.lol.attacker.net",
            "notapi.airplanes.live",
            "opensky-network.org.",
            "flightradar24.com",
            "api.flightaware.com",
            "adsbexchange.com",
        ] {
            assert!(!is_authorized_host(host), "{host} must not be authorized");
        }
    }

    #[test]
    fn the_prohibited_sources_are_not_on_the_list() {
        // Privacy rules 1.2–1.3, asserted rather than assumed.
        for host in AUTHORIZED_HOSTS {
            for prohibited in ["flightradar24", "flightaware", "adsbexchange"] {
                assert!(!host.contains(prohibited), "{host} is prohibited");
            }
        }
    }

    #[test]
    fn an_authorized_host_over_https_is_permitted() {
        assert!(HostPolicy::Authorized.permits(&url(
            "https://opensky-network.org/api/states/all?lamin=49&lomin=8&lamax=50&lomax=9"
        )));
    }

    #[test]
    fn an_authorized_host_without_tls_is_refused() {
        // Downgrading the token endpoint to cleartext would put the OAuth2 client secret on
        // the wire in the clear, so the scheme is part of the gate, not a detail of the URL.
        let error = HostPolicy::Authorized
            .check(&url("http://auth.opensky-network.org/token"))
            .expect_err("http is refused");
        assert_eq!(
            error,
            SourceError::Refused {
                reason: "http://auth.opensky-network.org is not an authorized origin \
                         (privacy rule 1.1)"
                    .to_owned()
            }
        );
    }

    #[test]
    fn an_unauthorized_host_is_refused() {
        let error = HostPolicy::Authorized
            .check(&url("https://www.flightradar24.com/api/feed"))
            .expect_err("prohibited host is refused");
        assert!(!error.is_transient(), "a refusal must never be retried");
        assert_eq!(
            error,
            SourceError::Refused {
                reason: "https://www.flightradar24.com is not an authorized origin \
                         (privacy rule 1.1)"
                    .to_owned()
            }
        );
    }

    #[test]
    fn a_refusal_does_not_echo_the_path_or_query() {
        // Privacy rule 7.1. The refusal is logged; the credential must not be.
        let error = HostPolicy::Authorized
            .check(&url("https://evil.example/steal?access_token=super-secret"))
            .expect_err("is refused");
        let SourceError::Refused { reason } = &error else {
            panic!("expected Refused, got {error:?}");
        };
        assert!(!reason.contains("super-secret"), "leaked: {reason}");
        assert!(!reason.contains("/steal"), "leaked: {reason}");
    }

    #[test]
    fn a_url_with_no_host_is_refused() {
        let error = HostPolicy::Authorized
            .check(&url("file:///etc/passwd"))
            .expect_err("is refused");
        assert_eq!(
            error,
            SourceError::Refused {
                reason: "a file URL with no host is not an authorized origin (privacy rule 1.1)"
                    .to_owned()
            }
        );
    }

    #[test]
    fn loopback_is_refused_by_the_production_policy() {
        // The escape hatch below must not be reachable from `HttpClient::new`.
        for target in [
            "http://127.0.0.1:8080/states",
            "http://localhost:8080/states",
            "http://[::1]:8080/states",
        ] {
            assert!(!HostPolicy::Authorized.permits(&url(target)), "{target}");
        }
    }

    #[test]
    fn the_test_policy_permits_loopback_and_still_refuses_the_rest() {
        assert!(HostPolicy::AuthorizedOrLoopback.permits(&url("http://127.0.0.1:8080/states")));
        assert!(HostPolicy::AuthorizedOrLoopback.permits(&url("http://localhost:8080/states")));
        assert!(HostPolicy::AuthorizedOrLoopback.permits(&url("http://[::1]:8080/states")));
        assert!(
            HostPolicy::AuthorizedOrLoopback.permits(&url("https://api.adsb.lol/v2/point/1/1/5"))
        );
        assert!(!HostPolicy::AuthorizedOrLoopback.permits(&url("http://192.168.1.10/states")));
        assert!(!HostPolicy::AuthorizedOrLoopback.permits(&url("https://www.flightradar24.com/")));
    }

    // ---- docs/10 §privacy: walk the crate's own URLs and assert membership ----

    /// Pulls URL literals out of Rust source, skipping comment lines.
    ///
    /// Comments are skipped so that citing a spec or a signup page in a doc comment is not a
    /// test failure — the guard is about where code *sends requests*, and a rule that punishes
    /// documentation is a rule someone eventually deletes.
    fn url_literals(code: &str) -> Vec<&str> {
        let mut found = Vec::new();
        for line in code.lines() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            let mut rest = line;
            while let Some(start) = rest.find("http") {
                if let Some(url) = leading_url(&rest[start..]) {
                    found.push(url);
                }
                rest = &rest[start + "http".len()..];
            }
        }
        found
    }

    /// Reads a URL off the front of `text`, or `None` if one does not start there.
    fn leading_url(text: &str) -> Option<&str> {
        if !(text.starts_with("http://") || text.starts_with("https://")) {
            return None;
        }
        let end = text
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(' | ')' | ',' | ';' | '>')
            })
            .unwrap_or(text.len());
        let url = text[..end].trim_end_matches(['.', ':']);
        (Url::parse(url).is_ok()).then_some(url)
    }

    /// Everything before this crate's own `mod tests`, which is allowed loopback URLs.
    ///
    /// A textual split rather than anything cleverer: `#[cfg(test)] mod tests` is where every
    /// file in this workspace puts its tests, and the alternative is parsing Rust.
    fn production_code(source: &str) -> String {
        let normalized = source.replace("\r\n", "\n");
        match normalized.split_once("#[cfg(test)]\nmod tests") {
            Some((code, _)) => code.to_owned(),
            None => normalized,
        }
    }

    fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("crate src is readable") {
            let path = entry.expect("directory entry is readable").path();
            if path.is_dir() {
                rust_files(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }

    #[test]
    fn the_literal_scanner_finds_urls_in_code_and_ignores_comments() {
        // The walk below is only as good as this; a scanner that silently finds nothing would
        // make it pass forever.
        let code = r#"
            //! See https://docs.example/spec for the format.
            // TODO: https://tracker.example/issue/7
            const BASE: &str = "https://api.adsb.lol/v2/point";
            let url = format!("{}/states", "https://opensky-network.org");
            let plain = "not a url at all";
        "#;
        assert_eq!(
            url_literals(code),
            [
                "https://api.adsb.lol/v2/point",
                "https://opensky-network.org"
            ]
        );
    }

    #[test]
    fn the_test_split_keeps_production_code_and_drops_the_test_module() {
        let source =
            "const A: u8 = 1;\n#[cfg(test)]\nmod tests {\n let u = \"http://127.0.0.1\";\n}\n";
        assert_eq!(production_code(source), "const A: u8 = 1;\n");
        assert!(url_literals(&production_code(source)).is_empty());
    }

    /// docs/10 §privacy: every URL this crate would request must be on the list.
    ///
    /// Stronger than walking declared base URLs, because it sees a URL hardcoded anywhere in
    /// the crate rather than only the ones an adapter remembered to register. It is a tripwire
    /// that arms itself as adapters land in 1.3–1.6: today the crate contains no request URL
    /// yet, which is why the scanner has its own test above.
    #[test]
    fn every_url_literal_in_this_crate_targets_an_authorized_host() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        assert!(
            !files.is_empty(),
            "found no sources under {}",
            src.display()
        );

        for file in files {
            let source = std::fs::read_to_string(&file).expect("source is readable");
            for literal in url_literals(&production_code(&source)) {
                let parsed = Url::parse(literal).expect("scanner yields parseable URLs");
                let host = parsed.host_str().unwrap_or_default();
                assert!(
                    is_authorized_host(host),
                    "{} requests {host}, which is not on AUTHORIZED_HOSTS — if this source is \
                     authorized, add it to the const with a DECISION_LOG entry (privacy 1.1)",
                    file.display(),
                );
            }
        }
    }
}
