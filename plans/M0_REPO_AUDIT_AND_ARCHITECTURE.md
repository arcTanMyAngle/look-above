# M0 — Repo & Architecture Setup

**Goal:** a clean cargo workspace where every later milestone has an obvious home, with CI,
config, and logging in place. Exit criteria: [../docs/11_ACCEPTANCE_CRITERIA.md](../docs/11_ACCEPTANCE_CRITERIA.md) §M0.
Constraining docs: 02 (ADRs), 07, 09 (contract shapes to stub).

## Workspace layout

```
Cargo.toml            # [workspace] members, shared lints, pinned shared deps
crates/
  core/               # types (StateVector, Icao24, BBox), geo math, sim, contracts — NO I/O deps
  ingest/             # LiveSource adapters, poller, budget, token cache (tokio, reqwest)
  store/              # rusqlite wrapper, migrations, writer thread
  render/             # wgpu pipelines, WGSL, glyph atlas, camera (no network, no DB)
  app/                # binary `look-above`: wiring, config, event loop, debug overlay
scripts/              # record_fixture.rs (M1), import helpers
tests/fixtures/       # trimmed recorded API responses (committed)
.github/workflows/ci.yml
config.example.toml
```

Dependency direction (enforced by review + `cargo tree` check in CI later):
`core` ← `ingest`, `store`, `render` ← `app`. `core` depends only on std + serde + rayon + thiserror.

## Checklist

- [x] 0.1 `cargo new` workspace + five crates; workspace-level `[lints]` (clippy pedantic
      subset agreed in ADR); `rust-toolchain.toml` pinning stable; edition 2024.
      *(2026-07-15: done — stable 1.96.0 pinned; lint subset in root Cargo.toml, see DECISION_LOG.)*
- [x] 0.2 Pin dependencies (exact versions, workspace `[workspace.dependencies]`):
      tokio (rt-multi-thread, macros), reqwest (json, rustls), serde/serde_json, rayon,
      crossbeam-channel, rusqlite (bundled), wgpu, winit, thiserror, anyhow (app only),
      tracing + tracing-subscriber. Record versions in DECISION_LOG.
      *(2026-07-15: done — versions + rationale in DECISION_LOG; `toml` added for 0.5 config.
      Verified: workspace builds, no OpenSSL/native-tls (rustls only), SQLite bundled,
      single raw-window-handle 0.6.2 shared by wgpu 30 + winit 0.30.)*
- [x] 0.3 `core`: define `StateVector`, `Icao24`, `CallSign`, `BBox`, `SourceId`, error types,
      and the `LiveSource`/`Store` traits from docs/09 (compile-only stubs, unit-testable
      newtype parsing: `Icao24::from_hex`).
      *(2026-07-15: done — `core::types` / `core::error` / `core::contracts`, 23 unit tests.
      `async-trait` added for dyn-compatible `LiveSource`; `RenderFeed` and serde derives
      deferred (not in scope / no consumer yet). Rationale in DECISION_LOG.)*
- [ ] 0.4 `core::geo`: haversine, bearing, destination-point, Web Mercator fwd/inv — with the
      golden-value unit tests from docs/10 §1 (this is real M0 code, it unblocks everything).
- [ ] 0.5 `app`: config loading (`config.toml` → struct via serde, env var overrides
      `LOOK_ABOVE_*`, defaults when absent); tracing init; `config.example.toml`; `.gitignore`
      (config.toml, target/, qa/, *.db).
- [ ] 0.6 `app`: winit window (title "Look Above", dark clear color via wgpu surface),
      resize + close handling, frame-stats stub in log.
- [ ] 0.7 CI: GitHub Actions — fmt --check, clippy -D warnings, test --workspace on
      windows-latest + ubuntu-latest.
- [ ] 0.8 Gate: run acceptance §M0, record results in CURRENT_STATUS, human review.

## Notes

- Keep `render` empty-ish in M0 (surface + clear only); the real pipeline is M2.
- No API calls anywhere in M0. `ingest` compiles with trait stubs only.
- Decisions already made — don't relitigate: ADRs 001–005 in docs/02.
