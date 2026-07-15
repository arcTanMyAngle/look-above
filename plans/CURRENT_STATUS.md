# Current Status

> The single source of truth for "where are we". Every session reads this first and updates
> it last. Keep the Now section ≤ 10 lines; move history to the log below.

## Now (updated 2026-07-15)

- **Phase:** M0 in progress — workspace skeleton compiles clean (fmt/clippy/test green).
- **Active milestone:** M0, item 0.1 done. Plan: [M0_REPO_AUDIT_AND_ARCHITECTURE.md](M0_REPO_AUDIT_AND_ARCHITECTURE.md)
- **Next action:** M0 checklist item 0.2 (pin workspace dependencies, record versions in DECISION_LOG).
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

- **2026-07-15** — M0 item 0.1: cargo workspace (resolver 3) + five crates
  (core/ingest/store/render/app), workspace lints (clippy all+pedantic, unwrap_used),
  rust-toolchain.toml pinned to 1.96.0, edition 2024 via workspace.package.
  fmt/clippy/test all green. Next: 0.2 (pin deps).

- **2026-07-14** — Repository scaffolded: README/CLAUDE/AGENTS, master prompt, docs 00–13,
  plans (M0–M2, status/decision/risk/next-actions), 7 agents, 3 skills. Initial commit.
  Next: M0 item 0.1.
