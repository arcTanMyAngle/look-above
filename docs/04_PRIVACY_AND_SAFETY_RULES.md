# 04 — Privacy and Safety Rules

These rules are **binding**. They override feature requests, milestone plans, and
convenience. Any code change that touches identification, data sourcing, or history must
cite the rule numbers it complies with in its commit message.

## 1. Source legitimacy

- **1.1** Data may only be fetched from sources on the allowlist in
  [../.claude/skills/authorized-aviation-sources/SKILL.md](../.claude/skills/authorized-aviation-sources/SKILL.md).
  Adding a source requires a decision-log entry confirming its terms permit free programmatic use.
- **1.2** Never scrape or reverse-engineer the web UIs or private APIs of FlightRadar24,
  FlightAware, ADS-B Exchange, Flightradar-like services, or airline websites. Their terms
  prohibit it. This includes "just this once" debugging fetches.
- **1.3** Respect every documented rate limit with margin (target ≤ 80% of allowance).
  Never rotate IPs, keys, or user agents to evade limits. Back off exponentially on 429/5xx.
- **1.4** Display required attribution (OpenSky requests citation/attribution; community
  aggregators are credited in the About screen and README).

## 2. Blocked and anonymized aircraft (LADD / PIA)

- **2.1** The FAA's LADD (Limiting Aircraft Data Displayed) and PIA (Privacy ICAO Address)
  programs exist so owners can opt out of public identification. Our authorized feeds
  already honor them. **We never attempt to undo that**: no cross-referencing an anonymized
  target against registries, historical data, or third-party APIs to recover identity.
- **2.2** If a feed provides a position with no identity (or a PIA hex), display it as
  "Unidentified" with position/altitude only. No enrichment lookups for such targets (this
  gates the adsbdb call in code — check before fetch).
- **2.3** No feature may exist to search, filter, list, or alert on specific tail numbers /
  hexes beyond what's visible on screen. Watching the sky, not watching a person.

## 3. Military and sensitive operations

- **3.1** Display military traffic only as the authorized feeds provide it; never infer,
  annotate, or highlight likely-military targets that feeds leave unlabeled.
- **3.2** No features that aggregate patterns over time for specific operators or areas
  (e.g., "activity heatmaps for airbase X"). Global density visualization (L0) is real-time
  only and operator-agnostic.

## 4. People

- **4.1** This is an aircraft tracker, not a people tracker. No feature may link aircraft to
  named private individuals.
- **4.2** No "celebrity jet" style tracking, alerting, or publishing — this category has
  documented harassment and safety problems, and several sources' terms prohibit it.
- **4.3** FAA registry imports (doc 03) drop/ignore owner name fields for individuals at
  display time; only aircraft type and operator/airline class are shown.

## 5. History (M5)

- **5.1** Position history is stored locally only, capped (default 24 h, max 7 days,
  user-configurable downward), and pruned automatically.
- **5.2** Rules 2.x apply retroactively: if an aircraft was anonymized live, its stored
  track stays anonymous in replay.
- **5.3** No export/sharing features for tracks in v1.

## 6. Safety-of-information

- **6.1** Emergency squawk display (7500/7600/7700) is passive visualization only — no
  notifications, no social features, no "spotted an emergency" export. We are not an
  alerting service and must never look like one.
- **6.2** The app displays a permanent footer note in the info card: data is unofficial,
  delayed, and unsuitable for operational, navigational, or safety use.

## 7. Secrets & local data

- **7.1** Credentials (OpenSky client id/secret) live in exactly three places, in precedence
  order: the `LOOK_ABOVE_OPENSKY_*` environment variables, gitignored `config.toml`, or the
  gitignored `credentials.json` that OpenSky's account page issues — read as-downloaded, and
  all-or-nothing, so a pair is never assembled from two sources (M1 item 1.3; DECISION_LOG
  2026-07-15). Never in code, logs, commits, or fixtures.
- **7.1a** Credential material is carried as `core::secret::SecretString`, whose `Debug` is
  redacted and which deliberately has no `Display`. `SecretString::expose` is the single
  audited route to a value: call it where the credential is *used*, never where one is
  logged, formatted, or put in an error message. Errors quoting a URL strip it first
  (`reqwest::Error::without_url`), since a token can ride in a query string.
- **7.2** Recorded test fixtures are scrubbed: real hexes/callsigns may remain (they're
  public data from authorized feeds) but any credential material or account metadata is removed.

## Enforcement in code review

The testing-agent and any reviewer check: new HTTP hosts against the allowlist (1.1–1.2),
identity lookups behind the anonymity gate (2.2), no tail-specific alert surfaces (2.3),
history caps (5.1). A PR violating any rule is rejected regardless of feature value.
