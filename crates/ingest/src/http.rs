//! The shared HTTP client every adapter builds on.
//!
//! Adapters do not construct their own [`reqwest::Client`]: the identifying User-Agent
//! and the request timeout that docs/09 mandates live here, so there is one place to get
//! them right and one place to audit. Adapters shape the request ([`HttpClient::get`] →
//! query params, auth) and hand it back to [`send_json`], which turns every failure into
//! the [`SourceError`] taxonomy the poller branches on.
//!
//! Being the one place every adapter passes through also makes this the place the host
//! allowlist is enforced ([`crate::allowlist`], privacy rule 1.1) — on the way out in
//! [`HttpClient::get`], and on every redirect hop.

pub mod backoff;

use std::time::Duration;

use look_above_core::error::SourceError;
use reqwest::{Client, RequestBuilder, Response, StatusCode, Url, header::HeaderMap, redirect};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::allowlist::HostPolicy;

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

/// Redirect hops we will follow, matching `reqwest`'s own default.
///
/// Restated because installing a custom redirect policy replaces the default limit rather
/// than adding to it — without this, a redirect loop between two authorized hosts would
/// spin until the 10 s timeout instead of stopping.
const MAX_REDIRECTS: usize = 10;

/// A configured [`reqwest::Client`], cloneable and shared across adapters.
///
/// Cloning is cheap and shares the connection pool — build one per process, not per
/// request, or every poll pays a fresh TLS handshake.
#[derive(Debug, Clone)]
pub struct HttpClient {
    inner: Client,
    hosts: HostPolicy,
}

impl HttpClient {
    /// Builds the client with the mandated User-Agent, timeout, and host allowlist.
    pub fn new() -> Result<Self, SourceError> {
        Self::build(REQUEST_TIMEOUT, HostPolicy::Authorized)
    }

    /// `pub(crate)` so sibling modules' tests can widen the policy to reach a loopback mock
    /// and still exercise the shipping client. Not public: outside this crate the only way
    /// to a client is [`new`](Self::new), which cannot be talked out of the allowlist.
    pub(crate) fn build(timeout: Duration, hosts: HostPolicy) -> Result<Self, SourceError> {
        let inner = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(timeout)
            .redirect(redirect_policy(hosts))
            .build()
            .map_err(|error| SourceError::Network {
                message: format!("could not build HTTP client: {}", describe(error)),
            })?;
        Ok(Self { inner, hosts })
    }

    /// Starts a GET the caller finishes (query params, auth) and passes to [`send_json`].
    ///
    /// The allowlist check happens here rather than in [`send_json`] because this is where
    /// a host enters: a [`RequestBuilder`] can gain query params and headers afterwards,
    /// but not a different host. Checking the parsed URL — not the string — is what makes
    /// the difference between a rule and a spelling of a rule.
    pub fn get(&self, url: &str) -> Result<RequestBuilder, SourceError> {
        Ok(self.inner.get(self.checked_url(url)?))
    }

    /// Starts a POST with a form-encoded body — the shape `OAuth2` token endpoints take.
    ///
    /// The only method here that sends a body, and the reason it exists is
    /// [`opensky::auth`](crate::opensky::auth): the client-credentials grant is a POST, and
    /// routing it through this type rather than a bare [`reqwest::Client`] is what keeps the
    /// allowlist a choke point instead of a suggestion. The credential goes in the *body*,
    /// never the query string — a URL reaches proxy logs and `reqwest`'s error `Display`;
    /// a body reaches neither (privacy rule 7.1).
    pub fn post_form<T: Serialize + ?Sized>(
        &self,
        url: &str,
        form: &T,
    ) -> Result<RequestBuilder, SourceError> {
        Ok(self.inner.post(self.checked_url(url)?).form(form))
    }

    /// Parses `url` and puts it through the allowlist, or refuses it.
    fn checked_url(&self, url: &str) -> Result<Url, SourceError> {
        let url = Url::parse(url).map_err(|error| SourceError::Refused {
            // `url::ParseError` describes the fault ("invalid IPv6 address") without
            // echoing the input, so this cannot leak a token from a query string.
            reason: format!("could not parse the URL: {error}"),
        })?;
        self.hosts.check(&url)?;
        Ok(url)
    }
}

/// Applies the allowlist to every redirect hop.
///
/// A 302 from an authorized host is still a URL we did not choose. `reqwest` follows
/// redirects by default, so without this the gate on the way out would be one `Location`
/// header away from irrelevant — an authorized-but-compromised source, or a captive
/// portal, could hand us anywhere. Refused hops [`stop`](redirect::Attempt::stop), which
/// surfaces the 3xx itself and lands in [`status_error`] as a `Refused`.
fn redirect_policy(hosts: HostPolicy) -> redirect::Policy {
    redirect::Policy::custom(move |attempt| {
        // `>` not `>=`: `previous()` counts the original request too, so `> MAX_REDIRECTS`
        // is what allows exactly that many hops — the same comparison `reqwest`'s own
        // `Policy::limited` makes.
        if attempt.previous().len() > MAX_REDIRECTS || !hosts.permits(attempt.url()) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    })
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
        // A redirect reaching the caller means [`redirect_policy`] declined to follow it:
        // the client follows the ones it is allowed to, so an unfollowed hop is our
        // refusal, not the source rejecting us. Named explicitly because falling into the
        // `Request` arm below would report "source rejected the request: HTTP 302", which
        // is precisely backwards. 304 is deliberately not here — it is a 3xx that means
        // "unchanged", and conditional requests are the import tooling's business.
        StatusCode::MOVED_PERMANENTLY
        | StatusCode::FOUND
        | StatusCode::SEE_OTHER
        | StatusCode::TEMPORARY_REDIRECT
        | StatusCode::PERMANENT_REDIRECT => SourceError::Refused {
            reason: format!(
                "HTTP {} to a host we may not follow, or past {MAX_REDIRECTS} hops \
                 (privacy rule 1.1)",
                status.as_u16()
            ),
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
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct Body {
        ok: bool,
    }

    /// The real client, with loopback added to the allowlist so it can reach a mock.
    ///
    /// That one widening is the whole difference from `HttpClient::new`, and it is what
    /// lets every test below exercise the shipping timeout, User-Agent, and redirect
    /// policy rather than a rehearsal of them. `refuses_an_unauthorized_host` uses the
    /// real constructor to prove the widening is test-only.
    fn client() -> HttpClient {
        HttpClient::build(REQUEST_TIMEOUT, HostPolicy::AuthorizedOrLoopback).expect("client builds")
    }

    /// Impatient client, for the two tests that want a failure rather than a reply.
    /// Everything else uses the real 10 s timeout: a mock on loopback answers in
    /// microseconds, and a tight deadline would only buy flakes on a loaded CI runner.
    fn impatient_client() -> HttpClient {
        HttpClient::build(Duration::from_millis(200), HostPolicy::AuthorizedOrLoopback)
            .expect("client builds")
    }

    /// Builds a GET the allowlist is expected to permit.
    fn get(client: &HttpClient, url: &str) -> RequestBuilder {
        client.get(url).expect("URL is allowed")
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
        send_json(get(&client(), &format!("{}/states", server.uri()))).await
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

        let body: Body = send_json(get(&client(), &format!("{}/states", server.uri())))
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
        let error = send_json::<Body>(get(
            &impatient_client(),
            &format!("{}/states", server.uri()),
        ))
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
        let error = send_json::<Body>(get(&impatient_client(), url))
            .await
            .expect_err("is an error");
        let SourceError::Network { message } = &error else {
            panic!("expected Network, got {error:?}");
        };
        assert!(!message.contains("super-secret"), "leaked: {message}");
        assert!(!message.contains("/states"), "leaked: {message}");
    }

    /// Privacy rule 1.1, on the client adapters are actually handed.
    ///
    /// The only test here that builds via `HttpClient::new`: everything else widens the
    /// policy to reach a mock, and this is what says that widening never ships.
    #[test]
    fn the_real_client_refuses_an_unauthorized_host() {
        let client = HttpClient::new().expect("client builds");
        let error = client
            .get("https://www.flightradar24.com/api/feed")
            .expect_err("prohibited host is refused");
        assert!(matches!(error, SourceError::Refused { .. }), "{error:?}");
        assert!(!error.is_transient());
        // And loopback — the escape hatch the tests use — is closed here too.
        assert!(client.get("http://127.0.0.1:8080/states").is_err());
        // An authorized host still builds.
        assert!(client.get("https://api.adsb.lol/v2/point/50/8/25").is_ok());
    }

    /// The gate must cover every method that can leave the process, not just `get`.
    ///
    /// `post_form` is the one that carries the `OAuth2` client secret, so a gap here would
    /// mail the credential to whatever host a bug named.
    #[test]
    fn post_form_is_gated_by_the_same_allowlist() {
        let client = HttpClient::new().expect("client builds");
        let form = [("client_secret", "super-secret")];

        let error = client
            .post_form("https://evil.example/token", &form)
            .expect_err("prohibited host is refused");
        let SourceError::Refused { reason } = &error else {
            panic!("expected Refused, got {error:?}");
        };
        assert!(!error.is_transient());
        assert!(!reason.contains("super-secret"), "leaked: {reason}");

        // Cleartext to an otherwise-authorized host would put the secret on the wire.
        assert!(
            client
                .post_form("http://auth.opensky-network.org/token", &form)
                .is_err(),
            "the OAuth2 grant must never go out over http"
        );

        // The real token endpoint still builds.
        assert!(
            client
                .post_form(
                    "https://auth.opensky-network.org/auth/realms/opensky-network/protocol/openid-connect/token",
                    &form,
                )
                .is_ok()
        );
    }

    /// The body actually arrives form-encoded — the shape an `OAuth2` endpoint requires.
    #[tokio::test]
    async fn post_form_sends_a_url_encoded_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string_contains("grant_type=client_credentials"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .expect(1)
            .mount(&server)
            .await;

        let form = [("grant_type", "client_credentials")];
        let request = client()
            .post_form(&format!("{}/token", server.uri()), &form)
            .expect("URL is allowed");
        assert_eq!(
            send_json::<Body>(request).await.expect("request succeeds"),
            Body { ok: true }
        );
    }

    #[test]
    fn an_unparseable_url_is_refused_rather_than_sent() {
        let error = client()
            .get("https://[not-an-address]/states?access_token=super-secret")
            .expect_err("is an error");
        let SourceError::Refused { reason } = &error else {
            panic!("expected Refused, got {error:?}");
        };
        assert!(!error.is_transient(), "a malformed URL never becomes valid");
        assert!(!reason.contains("super-secret"), "leaked: {reason}");
    }

    /// The gate on the way out is worth little if a `Location` header can walk around it.
    #[tokio::test]
    async fn a_redirect_off_the_allowlist_is_not_followed() {
        let server = server_returning(
            ResponseTemplate::new(302).insert_header("location", "https://www.flightradar24.com/"),
        )
        .await;
        let error = get_body(&server).await.expect_err("is an error");
        let SourceError::Refused { reason } = &error else {
            panic!("expected Refused, got {error:?}");
        };
        assert!(reason.contains("302"), "{reason}");
        assert!(!error.is_transient());
    }

    /// A redirect within the allowlist is ordinary and must still work.
    #[tokio::test]
    async fn a_redirect_to_a_permitted_host_is_followed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/states"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/states/v2", server.uri())),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/states/v2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .mount(&server)
            .await;

        assert_eq!(get_body(&server).await.expect("follows"), Body { ok: true });
    }

    /// Loops between permitted hosts stop at the hop limit instead of running to the timeout.
    #[tokio::test]
    async fn a_redirect_loop_stops_at_the_hop_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/states"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/states", server.uri())),
            )
            // The original request plus MAX_REDIRECTS hops, then we stop. Asserted on the
            // server, so a policy that quietly followed forever fails here rather than
            // hiding behind the 10 s timeout.
            .expect(u64::try_from(MAX_REDIRECTS).expect("fits") + 1)
            .mount(&server)
            .await;

        let error = get_body(&server).await.expect_err("is an error");
        assert!(matches!(error, SourceError::Refused { .. }), "{error:?}");
    }
}
