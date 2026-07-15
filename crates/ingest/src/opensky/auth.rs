//! `OpenSky` `OAuth2` client-credentials: token fetch, cache, and early refresh.
//!
//! `OpenSky` retired basic auth in 2025 and now accepts only the `OAuth2` *client
//! credentials* grant: POST the client id and secret to [`TOKEN_ENDPOINT`], receive an
//! access token good for ~30 minutes, send it as `Authorization: Bearer` on every API call.
//! If a tutorial or an old snippet shows a username and password, it is stale.
//!
//! [`OpenSkyAuth`] owns exactly one thing: producing a currently-valid token. It does not
//! know what the token is for — the `/states/all` adapter (item 1.4) is a separate concern,
//! and keeping them apart is what lets the refresh schedule below be tested without any
//! aircraft in sight.
//!
//! **Why 80%.** We fetch a replacement once a token is 80% through its life, which leaves
//! the last 20% as slack. That slack is the whole point: if the token endpoint is briefly
//! down, the *current* token is still valid, so a refresh failure costs a log line instead
//! of a poll cycle (see [`Configured::token`]). Refreshing at 100% would give the same
//! number of requests and none of the resilience.
//!
//! **No credentials is not an error.** `OpenSky` is the only source here that needs an
//! account; the community fallbacks (items 1.5–1.6) need none. So an unconfigured
//! `OpenSky` is a *disabled* source that the poller skips, not a failure that stops the
//! app — see [`OpenSkyAuth::disabled`].

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use look_above_core::error::SourceError;
use look_above_core::secret::SecretString;
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::http::{HttpClient, send_json};

/// `OpenSky`'s `OAuth2` token endpoint (authorized-aviation-sources skill).
///
/// A different host from the API itself, which is why the allowlist carries both.
pub const TOKEN_ENDPOINT: &str =
    "https://auth.opensky-network.org/auth/realms/opensky-network/protocol/openid-connect/token";

/// The fraction of a token's lifetime after which we fetch a replacement.
const REFRESH_AT: f64 = 0.8;

/// The longest TTL we will honor.
///
/// `OpenSky`'s tokens last 30 minutes, so this is far out of normal range and exists to be
/// a bound rather than a policy: `Instant + Duration` *panics* on overflow, and `expires_in`
/// is a number from the network. Clamping keeps a garbage response a parse problem instead
/// of a crash.
const MAX_TTL: Duration = Duration::from_hours(24);

/// An `OpenSky` API client id and secret, as issued by the account page.
#[derive(Debug, Clone)]
pub struct Credentials {
    pub client_id: SecretString,
    pub client_secret: SecretString,
}

impl Credentials {
    pub fn new(client_id: SecretString, client_secret: SecretString) -> Self {
        Self {
            client_id,
            client_secret,
        }
    }
}

/// A source of monotonic time.
///
/// Injected so the refresh schedule can be tested by advancing a clock rather than by
/// sleeping for 24 minutes. `Instant` rather than `SystemTime` on purpose: a token's life is
/// a duration, and a user correcting their wall clock must not expire it early or late.
pub trait Clock: fmt::Debug + Send + Sync {
    fn now(&self) -> Instant;
}

/// The real clock.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// The `OpenSky` source's authentication state: either credentialed, or deliberately off.
#[derive(Debug)]
pub struct OpenSkyAuth {
    /// `None` is the disabled state — see the module docs.
    inner: Option<Configured>,
}

#[derive(Debug)]
struct Configured {
    client: HttpClient,
    credentials: Credentials,
    token_endpoint: String,
    clock: Arc<dyn Clock>,
    cached: Mutex<Option<CachedToken>>,
}

/// A token and the two deadlines that govern it.
#[derive(Debug, Clone)]
struct CachedToken {
    token: SecretString,
    /// 80% through the token's life: fetch a replacement from here on.
    refresh_at: Instant,
    /// 100%: past this the token is worthless and a failure to refresh is fatal.
    expires_at: Instant,
}

impl OpenSkyAuth {
    /// Credentialed, using the real endpoint and the real clock.
    pub fn new(client: HttpClient, credentials: Credentials) -> Self {
        Self::build(
            client,
            credentials,
            TOKEN_ENDPOINT.to_owned(),
            Arc::new(SystemClock),
        )
    }

    /// No credentials: [`token`](Self::token) yields `Ok(None)` forever and nothing is sent.
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    /// [`new`](Self::new) or [`disabled`](Self::disabled), matching how configuration
    /// reports credentials — present or absent, never half (`app::config` rejects half a
    /// pair before we get here).
    pub fn from_optional(client: HttpClient, credentials: Option<Credentials>) -> Self {
        match credentials {
            Some(credentials) => Self::new(client, credentials),
            None => Self::disabled(),
        }
    }

    /// The one real constructor. Private so that the endpoint and clock overrides the tests
    /// need cannot become public API — the seam exists for testing, not for callers.
    fn build(
        client: HttpClient,
        credentials: Credentials,
        token_endpoint: String,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            inner: Some(Configured {
                client,
                credentials,
                token_endpoint,
                clock,
                cached: Mutex::new(None),
            }),
        }
    }

    /// Whether credentials were configured at all.
    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// A currently-valid bearer token, fetching or refreshing only when needed.
    ///
    /// `Ok(None)` means the source is disabled — the caller skips `OpenSky` and uses a
    /// fallback. It is not an error and must not be logged as one.
    pub async fn token(&self) -> Result<Option<SecretString>, SourceError> {
        match &self.inner {
            Some(configured) => configured.token().await.map(Some),
            None => Ok(None),
        }
    }
}

impl Configured {
    async fn token(&self) -> Result<SecretString, SourceError> {
        // The guard is held across the fetch on purpose: ten concurrent callers arriving at
        // a cold cache should cost one token request, not ten. That makes this `tokio`'s
        // Mutex rather than `std`'s — a `std` guard cannot be held across an await point.
        let mut cached = self.cached.lock().await;
        let now = self.clock.now();

        if let Some(token) = cached.as_ref()
            && now < token.refresh_at
        {
            return Ok(token.token.clone());
        }

        match self.fetch(now).await {
            Ok(fresh) => {
                let token = fresh.token.clone();
                *cached = Some(fresh);
                Ok(token)
            }
            // Between 80% and 100% of its life the cached token is still perfectly good.
            // Failing here would throw away the very margin the early refresh exists to
            // create, so a blip at the token endpoint costs a warning, not a poll cycle.
            Err(error) => match cached.as_ref().filter(|token| now < token.expires_at) {
                Some(token) => {
                    tracing::warn!(
                        %error,
                        "OpenSky token refresh failed; the cached token is still valid, \
                         so continuing with it"
                    );
                    Ok(token.token.clone())
                }
                None => Err(error),
            },
        }
    }

    async fn fetch(&self, now: Instant) -> Result<CachedToken, SourceError> {
        // A form body, not a query string: the secret belongs in the body, where it stays
        // out of proxy logs and out of `reqwest`'s error `Display` (privacy rule 7.1).
        let form = [
            ("grant_type", "client_credentials"),
            ("client_id", self.credentials.client_id.expose()),
            ("client_secret", self.credentials.client_secret.expose()),
        ];
        let request = self.client.post_form(&self.token_endpoint, &form)?;
        let response: TokenResponse = send_json(request).await?;
        response.into_cached(now)
    }
}

/// The token endpoint's reply. Fields we do not use (`scope`, `refresh_expires_in`, …) are
/// ignored rather than denied — this is Keycloak's shape, not ours, and a field appearing
/// there is not our business.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: SecretString,
    expires_in: u64,
    token_type: String,
}

impl TokenResponse {
    fn into_cached(self, now: Instant) -> Result<CachedToken, SourceError> {
        // Checked rather than assumed: we are about to put this in an `Authorization:
        // Bearer` header, and a token that is not a bearer token would be rejected by the
        // API with a 401 that looks exactly like bad credentials. Failing here names the
        // real fault.
        if !self.token_type.eq_ignore_ascii_case("bearer") {
            return Err(SourceError::Parse {
                message: format!(
                    "expected a bearer token, got token_type {:?}",
                    self.token_type
                ),
            });
        }
        if self.access_token.is_blank() {
            return Err(SourceError::Parse {
                message: "the token response carried an empty access_token".to_owned(),
            });
        }
        let ttl = Duration::from_secs(self.expires_in).min(MAX_TTL);
        if ttl.is_zero() {
            return Err(SourceError::Parse {
                message: "the token response expires_in is 0, so the token is already dead"
                    .to_owned(),
            });
        }
        Ok(CachedToken {
            token: self.access_token,
            refresh_at: now + ttl.mul_f64(REFRESH_AT),
            expires_at: now + ttl,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::allowlist::{HostPolicy, is_authorized_host};
    use crate::http::{REQUEST_TIMEOUT, USER_AGENT};

    /// `OpenSky`'s real TTL, so the arithmetic below is the arithmetic that will run.
    const TTL: Duration = Duration::from_mins(30);
    /// 80% of it — the moment a refresh becomes due.
    const REFRESH_DUE: Duration = Duration::from_mins(24);
    /// One second short of the refresh point. Written out rather than subtracted from
    /// [`REFRESH_DUE`]: `Duration` subtraction panics on underflow, so clippy rejects it.
    const JUST_BEFORE_REFRESH: Duration = Duration::from_secs(1439);

    /// A clock the test drives by hand.
    #[derive(Debug)]
    struct TestClock {
        base: Instant,
        offset: StdMutex<Duration>,
    }

    impl TestClock {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                base: Instant::now(),
                offset: StdMutex::new(Duration::ZERO),
            })
        }

        fn advance(&self, by: Duration) {
            *self.offset.lock().expect("clock lock") += by;
        }
    }

    impl Clock for TestClock {
        fn now(&self) -> Instant {
            self.base + *self.offset.lock().expect("clock lock")
        }
    }

    /// The real client, widened to reach a loopback mock — the same one-line escape hatch
    /// `http`'s own tests use, so every assertion below runs through the shipping
    /// User-Agent, timeout, and allowlist.
    fn client() -> HttpClient {
        HttpClient::build(REQUEST_TIMEOUT, HostPolicy::AuthorizedOrLoopback).expect("client builds")
    }

    fn credentials() -> Credentials {
        Credentials::new(
            SecretString::from("test-client-id"),
            SecretString::from("test-client-secret"),
        )
    }

    fn auth_against(server: &MockServer, clock: Arc<dyn Clock>) -> OpenSkyAuth {
        OpenSkyAuth::build(
            client(),
            credentials(),
            format!("{}/token", server.uri()),
            clock,
        )
    }

    fn token_body(access_token: &str, expires_in: u64) -> serde_json::Value {
        json!({
            "access_token": access_token,
            "expires_in": expires_in,
            "token_type": "Bearer",
        })
    }

    /// Mounts the token endpoint, expecting exactly `times` requests — asserted on drop, so
    /// a cache that quietly refetches fails the test rather than passing it slowly.
    async fn mock_token_endpoint(server: &MockServer, response: ResponseTemplate, times: u64) {
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(response)
            .expect(times)
            .mount(server)
            .await;
    }

    async fn expose_token(auth: &OpenSkyAuth) -> String {
        auth.token()
            .await
            .expect("token request succeeds")
            .expect("the source is enabled")
            .expose()
            .to_owned()
    }

    // --- The disabled state --------------------------------------------------------------

    #[tokio::test]
    async fn without_credentials_the_source_is_disabled_rather_than_failing() {
        let auth = OpenSkyAuth::disabled();
        assert!(!auth.is_enabled());
        assert!(
            auth.token()
                .await
                .expect("disabled is not an error")
                .is_none(),
            "a disabled source yields no token and no error — the poller skips it"
        );
    }

    #[tokio::test]
    async fn from_optional_maps_absent_credentials_onto_the_disabled_state() {
        assert!(!OpenSkyAuth::from_optional(client(), None).is_enabled());
        assert!(OpenSkyAuth::from_optional(client(), Some(credentials())).is_enabled());
    }

    // --- The token request itself ---------------------------------------------------------

    /// Asserts the grant on the wire: the right method, the right form fields, and the
    /// project's User-Agent. A response we parsed but never correctly asked for is the
    /// failure this catches.
    #[tokio::test]
    async fn the_first_call_posts_the_client_credentials_grant() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(header("user-agent", USER_AGENT))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string_contains("grant_type=client_credentials"))
            .and(body_string_contains("client_id=test-client-id"))
            .and(body_string_contains("client_secret=test-client-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_body("tok-1", 1800)))
            .expect(1)
            .mount(&server)
            .await;

        let auth = auth_against(&server, TestClock::new());
        assert_eq!(expose_token(&auth).await, "tok-1");
    }

    #[tokio::test]
    async fn a_fresh_token_is_cached_rather_than_refetched() {
        let server = MockServer::start().await;
        mock_token_endpoint(
            &server,
            ResponseTemplate::new(200).set_body_json(token_body("tok-1", 1800)),
            1,
        )
        .await;

        let auth = auth_against(&server, TestClock::new());
        for _ in 0..5 {
            assert_eq!(expose_token(&auth).await, "tok-1");
        }
        // `expect(1)` above: five calls, one request.
    }

    /// The item's headline: refresh at 80% of TTL, not before, not at expiry.
    #[tokio::test]
    async fn the_token_is_refreshed_at_eighty_percent_of_its_life() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_body("tok-1", 1800)))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_body("tok-2", 1800)))
            .mount(&server)
            .await;

        let clock = TestClock::new();
        let auth = auth_against(&server, clock.clone());

        assert_eq!(expose_token(&auth).await, "tok-1");

        // One second short of 80%: still the original.
        clock.advance(JUST_BEFORE_REFRESH);
        assert_eq!(
            expose_token(&auth).await,
            "tok-1",
            "a token 79.9% through its life is still fresh"
        );

        // At 80%: replaced, while the old one is still valid.
        clock.advance(Duration::from_secs(1));
        assert_eq!(
            expose_token(&auth).await,
            "tok-2",
            "at 80% of TTL the token must be replaced"
        );
    }

    // --- Failure handling ------------------------------------------------------------------

    #[tokio::test]
    async fn rejected_credentials_surface_as_auth_and_are_not_retried() {
        let server = MockServer::start().await;
        mock_token_endpoint(&server, ResponseTemplate::new(401), 1).await;

        let auth = auth_against(&server, TestClock::new());
        let error = auth.token().await.expect_err("401 is an error");
        assert!(matches!(error, SourceError::Auth { .. }), "{error:?}");
        assert!(
            !error.is_transient(),
            "retrying rejected credentials only burns budget"
        );
    }

    /// The reason the refresh is early at all: the slack must actually be used.
    #[tokio::test]
    async fn a_failed_refresh_falls_back_to_the_still_valid_cached_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_body("tok-1", 1800)))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let clock = TestClock::new();
        let auth = auth_against(&server, clock.clone());
        assert_eq!(expose_token(&auth).await, "tok-1");

        // Refresh is due and the endpoint is down — but the token has 20% of its life left.
        clock.advance(REFRESH_DUE);
        assert_eq!(
            expose_token(&auth).await,
            "tok-1",
            "a token endpoint blip inside the slack window must not cost a poll cycle"
        );
    }

    #[tokio::test]
    async fn a_failed_refresh_after_expiry_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(token_body("tok-1", 1800)))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let clock = TestClock::new();
        let auth = auth_against(&server, clock.clone());
        assert_eq!(expose_token(&auth).await, "tok-1");

        // Past 100%: the cached token is worthless, so the failure is the answer.
        clock.advance(TTL);
        let error = auth
            .token()
            .await
            .expect_err("an expired token cannot be reused");
        assert_eq!(error, SourceError::Server { status: 503 });
    }

    // --- Response validation -----------------------------------------------------------------

    #[tokio::test]
    async fn a_non_bearer_token_type_is_a_parse_error() {
        let server = MockServer::start().await;
        mock_token_endpoint(
            &server,
            ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "tok-1",
                "expires_in": 1800,
                "token_type": "mac",
            })),
            1,
        )
        .await;

        let auth = auth_against(&server, TestClock::new());
        let error = auth
            .token()
            .await
            .expect_err("a non-bearer token is unusable");
        let SourceError::Parse { message } = &error else {
            panic!("expected Parse, got {error:?}");
        };
        assert!(message.contains("bearer"), "{message}");
    }

    #[tokio::test]
    async fn the_documented_bearer_spelling_is_accepted_in_any_case() {
        for spelling in ["Bearer", "bearer", "BEARER"] {
            let server = MockServer::start().await;
            mock_token_endpoint(
                &server,
                ResponseTemplate::new(200).set_body_json(json!({
                    "access_token": "tok-1",
                    "expires_in": 1800,
                    "token_type": spelling,
                })),
                1,
            )
            .await;

            let auth = auth_against(&server, TestClock::new());
            assert_eq!(expose_token(&auth).await, "tok-1", "token_type: {spelling}");
        }
    }

    #[tokio::test]
    async fn a_blank_or_dead_token_is_a_parse_error() {
        let cases = [
            (token_body("", 1800), "empty access_token"),
            (token_body("tok-1", 0), "expires_in is 0"),
        ];
        for (body, label) in cases {
            let server = MockServer::start().await;
            mock_token_endpoint(&server, ResponseTemplate::new(200).set_body_json(body), 1).await;

            let auth = auth_against(&server, TestClock::new());
            let error = auth.token().await.expect_err("is an error");
            assert!(
                matches!(error, SourceError::Parse { .. }),
                "{label}: {error:?}"
            );
        }
    }

    /// `Instant + Duration` panics on overflow, and `expires_in` comes off the network.
    #[tokio::test]
    async fn an_absurd_ttl_is_clamped_rather_than_overflowing_the_clock() {
        let server = MockServer::start().await;
        mock_token_endpoint(
            &server,
            ResponseTemplate::new(200).set_body_json(token_body("tok-1", u64::MAX)),
            1,
        )
        .await;

        let auth = auth_against(&server, TestClock::new());
        assert_eq!(
            expose_token(&auth).await,
            "tok-1",
            "a nonsense TTL must be clamped, never panic"
        );
    }

    // --- Privacy and allowlist ---------------------------------------------------------------

    /// Privacy rule 7.1, on the type that actually holds the token.
    #[tokio::test]
    async fn neither_the_token_nor_the_secret_reaches_debug_output() {
        let server = MockServer::start().await;
        mock_token_endpoint(
            &server,
            ResponseTemplate::new(200).set_body_json(token_body("super-secret-token", 1800)),
            1,
        )
        .await;

        let auth = auth_against(&server, TestClock::new());
        assert_eq!(expose_token(&auth).await, "super-secret-token");

        let rendered = format!("{auth:?}");
        assert!(
            !rendered.contains("super-secret-token"),
            "the cached token leaked into Debug: {rendered}"
        );
        assert!(
            !rendered.contains("test-client-secret"),
            "the client secret leaked into Debug: {rendered}"
        );
    }

    #[test]
    fn the_token_endpoint_is_the_documented_one_and_is_authorized() {
        assert_eq!(
            TOKEN_ENDPOINT,
            "https://auth.opensky-network.org/auth/realms/opensky-network/protocol/\
             openid-connect/token"
        );
        let host = reqwest::Url::parse(TOKEN_ENDPOINT)
            .expect("the endpoint parses")
            .host_str()
            .expect("the endpoint has a host")
            .to_owned();
        assert!(is_authorized_host(&host), "{host} must be on the allowlist");
    }

    /// The production client must refuse to send the grant anywhere but `OpenSky`, whatever a
    /// caller passes as the endpoint.
    #[tokio::test]
    async fn the_real_client_will_not_post_the_grant_to_an_unauthorized_host() {
        let auth = OpenSkyAuth::build(
            HttpClient::new().expect("client builds"),
            credentials(),
            "https://evil.example/token".to_owned(),
            TestClock::new(),
        );
        let error = auth
            .token()
            .await
            .expect_err("an unauthorized host is refused");
        let SourceError::Refused { reason } = &error else {
            panic!("expected Refused, got {error:?}");
        };
        assert!(!reason.contains("test-client-secret"), "leaked: {reason}");
    }

    // --- The real OpenSky ---------------------------------------------------------------------

    /// The one test here that talks to the real `OpenSky`.
    ///
    /// `#[ignore]` because it needs credentials and a network, and CI has neither — but the
    /// reason it exists is that *every other test in this file is a mock*. Mocks prove we
    /// parse what we believe `OpenSky` sends; this proves the belief. It checks the two
    /// assumptions the code is built on and cannot verify locally: that these credentials
    /// are accepted at all, and that the TTL really is the ~30 minutes the refresh schedule
    /// is tuned for.
    ///
    /// It costs no credits — the ledger meters `/states/*`, not the token endpoint — so it
    /// is safe to run on demand:
    ///
    /// ```text
    /// LOOK_ABOVE_OPENSKY_CLIENT_ID=… LOOK_ABOVE_OPENSKY_CLIENT_SECRET=… \
    ///     cargo test -p look-above-ingest -- --ignored live_opensky
    /// ```
    ///
    /// Nothing here prints the token or the secret (docs/06: never paste raw API responses
    /// into a log or a transcript) — only their shape.
    #[tokio::test]
    #[ignore = "hits the real OpenSky token endpoint; needs credentials in the environment"]
    async fn live_opensky_issues_a_usable_bearer_token() {
        let (Ok(client_id), Ok(client_secret)) = (
            std::env::var("LOOK_ABOVE_OPENSKY_CLIENT_ID"),
            std::env::var("LOOK_ABOVE_OPENSKY_CLIENT_SECRET"),
        ) else {
            panic!(
                "set LOOK_ABOVE_OPENSKY_CLIENT_ID and LOOK_ABOVE_OPENSKY_CLIENT_SECRET to \
                 run this test"
            );
        };

        // The real client and the real endpoint: no loopback widening, no injected clock.
        let auth = OpenSkyAuth::new(
            HttpClient::new().expect("client builds"),
            Credentials::new(
                SecretString::from(client_id),
                SecretString::from(client_secret),
            ),
        );

        let token = auth
            .token()
            .await
            .expect("OpenSky accepts our credentials")
            .expect("the source is enabled");
        assert!(!token.is_blank(), "OpenSky returned a blank token");

        // The TTL the whole refresh schedule is tuned for, checked against the real thing
        // rather than against the docs. A token life that drifted far from 30 minutes would
        // not fail anything locally — it would just quietly change how often we refresh.
        let configured = auth.inner.as_ref().expect("enabled");
        let cached = configured.cached.lock().await;
        let cached = cached.as_ref().expect("a token was cached");
        let ttl = cached.expires_at - configured.clock.now();
        assert!(
            ttl > Duration::from_mins(1),
            "a token valid for {ttl:?} is too short to be useful"
        );
        assert!(
            ttl <= MAX_TTL,
            "a token valid for {ttl:?} exceeds the clamp"
        );
        assert!(
            cached.refresh_at < cached.expires_at,
            "the refresh must fall inside the token's life"
        );
        eprintln!(
            "live OpenSky token: {} chars, TTL {} s, refresh in {} s",
            token.expose().len(),
            ttl.as_secs(),
            (cached.refresh_at - configured.clock.now()).as_secs(),
        );
    }

    /// Ten callers, one cold cache, one request — the guard is held across the fetch.
    #[tokio::test]
    async fn concurrent_callers_share_a_single_token_request() {
        let server = MockServer::start().await;
        mock_token_endpoint(
            &server,
            ResponseTemplate::new(200)
                .set_body_json(token_body("tok-1", 1800))
                .set_delay(Duration::from_millis(50)),
            1,
        )
        .await;

        let auth = Arc::new(auth_against(&server, TestClock::new()));
        let mut handles = Vec::new();
        for _ in 0..10 {
            let auth = Arc::clone(&auth);
            handles.push(tokio::spawn(async move { expose_token(&auth).await }));
        }
        for handle in handles {
            assert_eq!(handle.await.expect("task joins"), "tok-1");
        }
        // `expect(1)`: a per-caller fetch would be ten requests and a thundering herd on
        // every cold start.
    }
}
