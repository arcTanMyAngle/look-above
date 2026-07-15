# Risk Register

Reviewed at every milestone gate; likelihood/impact are High/Med/Low. New risks get an ID
and an owner-visible mitigation before the gate closes.

| ID | Risk | L | I | Mitigation | Trigger / early warning |
|---|---|---|---|---|---|
| R1 | **OpenSky changes limits or auth again** (they moved to OAuth2 in 2025; free credit policy can change) | M | H | Adapter isolation behind `LiveSource`; two no-key fallbacks always maintained and failover-tested at every gate | 401s with valid creds; announcement on opensky-network.org |
| R2 | **Community aggregators (adsb.lol / airplanes.live) throttle, change response shape, or disappear** — they're volunteer-run | M | M | Two independent fallbacks, shared readsb parser tested per-source; fixtures pin expected shapes so drift is caught in tests when re-recorded | 429s/timeouts in source_status; fixture re-record diffs |
| R3 | **wgpu/winit breaking API churn** between versions | H | M | Versions pinned; upgrades only at milestone gates as scoped tasks (ADR-003) | cargo update dry-run diffs at gates |
| R4 | **Windows GPU driver quirks on integrated graphics** (surface loss, DX12 vs Vulkan differences) | M | M | Handle surface-lost/outdated in the render loop from M0; test on the owner's actual hardware at every visual gate; wgpu backend override flag in config | Validation errors / device-lost logs |
| R5 | **Free-tier rate budgets too tight for the UX we spec'd** (global mode especially) | M | H | Budget controller degrades cadence gracefully; L0 global mode designed around coarse update rates (density, not per-plane fidelity); measure real credit burn at M1 gate before M4 commits | M1 gate credit numbers > 60% of pro-rated budget |
| R6 | **Dead-reckoning looks wrong in edge cases** (turns, go-arounds, data gaps) making the core pillar unconvincing | M | H | Correction-blend spec with tests; visual QA has explicit turn/gap scenarios; skill documents the math so it's reviewable | QA §L2-core motion items failing |
| R7 | **Scope creep** (own ADS-B receiver, alerts, web export…) derails milestones | H | M | Non-goals in docs/00; NEXT_ACTIONS "post-v1 parking lot"; gates require human review | Checklist items appearing mid-milestone |
| R8 | **AI-session drift**: sessions re-decide settled questions or violate privacy/source rules | M | H | Binding docs (04), allowlist regression test, decision log, master-prompt stop rules; gates re-verify privacy tests | Decision log entries contradicting ADRs; allowlist test edits |
| R9 | **Privacy regression** (enrichment gate bypassed, retention cap broken) | L | H | Dedicated regression tests (docs/10 §privacy) run in CI; commit-message rule citations for privacy-touching changes (docs/04) | Those tests failing or being modified |
| R10 | **Single-maintainer bus factor / motivation dip** | M | M | Docs-first structure means any session (human or AI) can resume from CURRENT_STATUS cold; milestones sized to deliver visible wins early (M2 is watchable) | CURRENT_STATUS stale > 1 month |

## Accepted (no action)

- SQLite single-writer ceiling — fine for one desktop process (ADR-004).
- No macOS/Linux polish in v1 — CI builds on Linux keep the door open; not a v1 target.
