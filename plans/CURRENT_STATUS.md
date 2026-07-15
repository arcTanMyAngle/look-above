# Current Status

> The single source of truth for "where are we". Every session reads this first and updates
> it last. Keep the Now section ≤ 10 lines; move history to the log below.

## Now (updated 2026-07-15)

- **Phase:** M0 in progress — `core` vocabulary + crate seams defined (fmt/clippy/test green).
- **Active milestone:** M0, items 0.1–0.3 done. Plan: [M0_REPO_AUDIT_AND_ARCHITECTURE.md](M0_REPO_AUDIT_AND_ARCHITECTURE.md)
- **Next action:** M0 checklist item 0.4 (`core::geo`: haversine, bearing, destination-point,
  Web Mercator fwd/inv, with the golden-value unit tests from docs/10 §1).
- **Blockers:** none for M0. Before M1 item 1.3, the owner must create a free OpenSky
  account + API client (see [NEXT_ACTIONS.md](NEXT_ACTIONS.md) #1).
- **Decisions pending:** none — ADRs 001–005 accepted (docs/02).

## Gate record

| Milestone | Status | Evidence |
|---|---|---|
| M0 | not started | — |
| M1 | not started | — |
| M2 | not started | — |
| M3–M6 | not started (plan files written at preceding gates) | — |

## Session log (newest first)

- **2026-07-15** — M0 item 0.3: `core` types + contracts — `core::types` (StateVector, Icao24,
  CallSign, BBox, SourceId, UnixSeconds), `core::error` (SourceError/StoreError, backend-agnostic),
  `core::contracts` (LiveSource, Store, RegionQuery, AircraftMeta, Airport, AirportSize).
  Added `async-trait` 0.1.89 (proc-macro only; needed for a dyn-compatible `LiveSource`).
  Icao24 stores bytes (case-safe Eq/Hash) and rejects readsb `~`-prefixed non-ICAO addresses;
  BBox validates bounds and refuses antimeridian spans. Deferred: `RenderFeed` (M2 shapes),
  serde derives (no consumer yet). 23 unit tests; fmt/clippy/test green. Next: 0.4.

- **2026-07-15** — M0 item 0.2: workspace dependency pins (table + rationale in DECISION_LOG).
  Full `major.minor.patch` + committed Cargo.lock for reproducibility; `=` pins only on
  wgpu 30.0.0 / winit 0.30.13 per ADR-003. winit held at stable 0.30.13 (0.31 is beta).
  Verified wgpu+winit share raw-window-handle 0.6.2, tree is rustls-only (no OpenSSL), and
  SQLite is bundled. Deps wired into owning crates; build/fmt/clippy/test green. Next: 0.3.

- **2026-07-15** — M0 item 0.1: cargo workspace (resolver 3) + five crates
  (core/ingest/store/render/app), workspace lints (clippy all+pedantic, unwrap_used),
  rust-toolchain.toml pinned to 1.96.0, edition 2024 via workspace.package.
  fmt/clippy/test all green. Next: 0.2 (pin deps).

- **2026-07-14** — Repository scaffolded: README/CLAUDE/AGENTS, master prompt, docs 00–13,
  plans (M0–M2, status/decision/risk/next-actions), 7 agents, 3 skills. Initial commit.
  Next: M0 item 0.1.
