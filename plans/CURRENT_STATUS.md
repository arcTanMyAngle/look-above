# Current Status

> The single source of truth for "where are we". Every session reads this first and updates
> it last. Keep the Now section ≤ 10 lines; move history to the log below.

## Now (updated 2026-07-15)

- **Phase:** M0 in progress — workspace + pinned deps build clean (fmt/clippy/test green).
- **Active milestone:** M0, items 0.1–0.2 done. Plan: [M0_REPO_AUDIT_AND_ARCHITECTURE.md](M0_REPO_AUDIT_AND_ARCHITECTURE.md)
- **Next action:** M0 checklist item 0.3 (`core`: StateVector, Icao24, CallSign, BBox, SourceId,
  error types, `LiveSource`/`Store` trait stubs from docs/09; unit-test `Icao24::from_hex`).
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
