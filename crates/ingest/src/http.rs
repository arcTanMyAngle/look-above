//! The shared HTTP client every adapter builds on.
//!
//! Adapters do not construct their own [`reqwest::Client`]: the identifying User-Agent
//! and the request timeout that docs/09 mandates live here, so there is one place to get
//! them right and one place to audit. Adapters shape the request ([`HttpClient::get`] →
//! query params, auth) and hand it back to [`send_json`], which turns every failure into
//! the [`SourceError`] taxonomy the poller branches on.

pub mod backoff;

use std::time::Duration;

use look_above_core::error::SourceError;
use reqwest::{Client, RequestBuilder, Response, StatusCode, header::HeaderMap};
use serde::de::DeserializeOwned;

/// Identifies this project to every source we contact (docs/09).
///
/// The version tracks the workspace version, so a source operator reading their logs can
/// tell our releases apart.
pub const USER_AGENT: &str = concat!(
    "look-above/",
    env!("CARGO_PKG_VERSION"),
    " (github.com/arcTanMyAngle/look-above)"
);

/// Per-request ceiling, applied to the whole request→body round trip (docs/09).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// A configured [`reqwest::Client`], cloneable and shared across adapters.
///
/// Cloning is cheap and shares the connection pool — build one per process, not per
/// request, or every poll pays a fresh TLS handshake.
#[derive(Debug, Clone)]
pub struct HttpClient {
    inner: Client,
}

impl HttpClient {
    /// Builds the client with the mandated User-Agent and timeout.
    pub fn new() -> Result<Self, SourceError> {
        Self::build(REQUEST_TIMEOUT)
    }

    fn build(timeout: Duration) -> Result<Self, SourceError> {
        let inner = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(timeout)
            .build()
            .map_err(|error| SourceError::Network {
                message: format!("could not build HTTP client: {}", describe(error)),
            })?;
        Ok(Self { inner })
    }

    /// Starts a GET the caller finishes (query params, auth) and passes to [`send_json`].
    pub fn get(&self, url: &str) -> RequestBuilder {
        self.inner.get(url)
    }
}

/// Sends `request` and decodes a successful JSON body.
///
/// Every outcome maps onto [`SourceError`]: see [`status_error`] for the HTTP side and
/// [`transport_error`] for the socket side.
pub async fn send_json<T: DeserializeOwned>(request: RequestBuilder) -> Result<T, SourceError> {
    let response = request.send().await.map_err(transport_error)?;
    if let Some(error) = status_error(&response) {
        return Err(error);
    }
    response
        .json::<T>()
        .await
        .map_err(|error| SourceError::Parse {
            message: describe(error),
        })
}

/// Maps a non-success status onto the taxonomy; `None` means the response is good.
///
/// The split that matters is retryable vs. not (see [`SourceError::is_transient`]): 5xx
/// and 429 are worth coming back for, while a 400 or 404 means our request is wrong or
/// the endpoint moved — retrying that just burns budget on a bug, so it surfaces as
/// `Request` and lets the poller fail over.
fn status_error(response: &Response) -> Option<SourceError> {
    let status = response.status();
    if status.is_success() {
        return None;
    }
    Some(match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => SourceError::Auth {
            message: format!("source rejected our credentials (HTTP {})", status.as_u16()),
        },
        StatusCode::TOO_MANY_REQUESTS => SourceError::RateLimited {
            retry_after: retry_after(response.headers()),
        },
        _ if status.is_server_error() => SourceError::Server {
            status: status.as_u16(),
        },
        _ => SourceError::Request {
            status: status.as_u16(),
        },
    })
}

/// Maps a transport-level failure onto the taxonomy.
fn transport_error(error: reqwest::Error) -> SourceError {
    let decode = error.is_decode();
    let message = describe(error);
    if decode {
        SourceError::Parse { message }
    } else {
        SourceError::Network { message }
    }
}

/// Reads `Retry-After` as delta-seconds.
///
/// RFC 9110 also permits an HTTP-date, which we deliberately do not parse: it would cost
/// a date-parsing dependency to serve a form no aviation source here sends, and an
/// unreadable header is not a failure — the caller falls back to the exponential
/// schedule, which is what [`backoff::retry_delay`] does with `None`.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// Renders a `reqwest::Error` for a message field, with the URL stripped.
///
/// `reqwest`'s `Display` includes the failing URL, and privacy rule 7.1 forbids
/// credentials in logs — a source that takes a token as a query parameter would put one
/// in every error string. The poller knows which `SourceId` it was calling, so the URL
/// adds nothing the log doesn't already have.
fn describe(error: reqwest::Error) -> String {
    error.without_url().to_string()
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct Body {
        ok: bool,
    }

    /// The real thing — the client adapters will actually get.
    fn client() -> HttpClient {
        HttpClient::new().expect("client builds")
    }

    /// Impatient client, for the two tests that want a failure rather than a reply.
    /// Everything else uses the real 10 s timeout: a mock on loopback answers in
    /// microseconds, and a tight deadline would only buy flakes on a loaded CI runner.
    fn impatient_client() -> HttpClient {
        HttpClient::build(Duration::from_millis(200)).expect("client builds")
    }

    /// Mounts a single GET /states responder and returns the server.
    async fn server_returning(response: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/states"))
            .respond_with(response)
            .mount(&server)
            .await;
        server
    }

    async fn get_body(server: &MockServer) -> Result<Body, SourceError> {
        send_json(client().get(&format!("{}/states", server.uri()))).await
    }

    #[test]
    fn user_agent_names_the_project_version_and_repo() {
        assert_eq!(
            USER_AGENT,
            format!(
                "look-above/{} (github.com/arcTanMyAngle/look-above)",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn request_timeout_is_the_documented_ten_seconds() {
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(10));
    }

    /// The constant above only proves what we wrote down; this proves what goes on the wire.
    #[tokio::test]
    async fn every_request_carries_the_user_agent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/states"))
            .and(header("user-agent", USER_AGENT))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .expect(1)
            .mount(&server)
            .await;

        let body: Body = send_json(client().get(&format!("{}/states", server.uri())))
            .await
            .expect("request succeeds");
        assert_eq!(body, Body { ok: true });
        // Mock::expect(1) is asserted on drop: a missing User-Agent fails the match and
        // the call above would already have returned Request { status: 404 }.
    }

    #[tokio::test]
    async fn success_decodes_the_json_body() {
        let server = server_returning(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})),
        )
        .await;
        assert_eq!(get_body(&server).await.expect("decodes"), Body { ok: true });
    }

    #[tokio::test]
    async fn rejected_credentials_map_to_auth() {
        for status in [401, 403] {
            let server = server_returning(ResponseTemplate::new(status)).await;
            let error = get_body(&server).await.expect_err("is an error");
            assert!(
                matches!(error, SourceError::Auth { .. }),
                "HTTP {status} gave {error:?}"
            );
            assert!(!error.is_transient(), "HTTP {status} must not be retried");
        }
    }

    #[tokio::test]
    async fn rate_limit_carries_retry_after_seconds() {
        let server =
            server_returning(ResponseTemplate::new(429).insert_header("retry-after", "30")).await;
        assert_eq!(
            get_body(&server).await.expect_err("is an error"),
            SourceError::RateLimited {
                retry_after: Some(Duration::from_secs(30))
            }
        );
    }

    #[tokio::test]
    async fn rate_limit_without_a_usable_retry_after_falls_back_to_backoff() {
        // Bare 429, an HTTP-date we don't parse, and outright garbage all mean "we have
        // no floor from the source" — never "don't back off".
        let cases: [Option<&str>; 3] = [
            None,
            Some("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some("soon-ish"),
        ];
        for header_value in cases {
            let mut response = ResponseTemplate::new(429);
            if let Some(value) = header_value {
                response = response.insert_header("retry-after", value);
            }
            let server = server_returning(response).await;
            assert_eq!(
                get_body(&server).await.expect_err("is an error"),
                SourceError::RateLimited { retry_after: None },
                "retry-after: {header_value:?}"
            );
        }
    }

    #[tokio::test]
    async fn upstream_failure_maps_to_server_and_is_retryable() {
        for status in [500, 502, 503] {
            let server = server_returning(ResponseTemplate::new(status)).await;
            let error = get_body(&server).await.expect_err("is an error");
            assert_eq!(error, SourceError::Server { status });
            assert!(error.is_transient(), "HTTP {status} should be retried");
        }
    }

    #[tokio::test]
    async fn our_own_bad_request_maps_to_request_and_is_not_retryable() {
        for status in [400, 404, 410] {
            let server = server_returning(ResponseTemplate::new(status)).await;
            let error = get_body(&server).await.expect_err("is an error");
            assert_eq!(error, SourceError::Request { status });
            assert!(!error.is_transient(), "HTTP {status} must not be retried");
        }
    }

    #[tokio::test]
    async fn unreadable_body_maps_to_parse() {
        let server =
            server_returning(ResponseTemplate::new(200).set_body_string("not json at all")).await;
        let error = get_body(&server).await.expect_err("is an error");
        assert!(matches!(error, SourceError::Parse { .. }), "{error:?}");
        assert!(!error.is_transient(), "a re-fetch returns the same bytes");
    }

    /// Proves the timeout is actually applied, not just stored in a constant.
    ///
    /// The 200 ms client stands in for the 10 s one here — the mechanism under test is
    /// `Client::timeout` being wired at all, and asserting it at the real value would
    /// mean a ten-second test.
    #[tokio::test]
    async fn a_hanging_source_times_out_as_network() {
        let server = server_returning(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": true}))
                .set_delay(Duration::from_secs(30)),
        )
        .await;
        let error = send_json::<Body>(impatient_client().get(&format!("{}/states", server.uri())))
            .await
            .expect_err("is an error");
        assert!(matches!(error, SourceError::Network { .. }), "{error:?}");
        assert!(error.is_transient(), "a timeout is worth retrying");
    }

    /// Privacy rule 7.1: a token in a query string must not survive into a log line.
    ///
    /// Port 1 on loopback refuses instantly: no DNS, no waiting, and — unlike releasing a
    /// `MockServer`'s port — nothing a parallel test can bind underneath us.
    #[tokio::test]
    async fn error_messages_do_not_echo_the_url() {
        let url = "http://127.0.0.1:1/states?access_token=super-secret";
        let error = send_json::<Body>(impatient_client().get(url))
            .await
            .expect_err("is an error");
        let SourceError::Network { message } = &error else {
            panic!("expected Network, got {error:?}");
        };
        assert!(!message.contains("super-secret"), "leaked: {message}");
        assert!(!message.contains("/states"), "leaked: {message}");
    }
}
