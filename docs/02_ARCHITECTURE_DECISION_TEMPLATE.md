# 02 — Architecture Decision Records

New ADRs are appended here using the template below and cross-referenced from
[../plans/DECISION_LOG.md](../plans/DECISION_LOG.md).

## Template

```markdown
## ADR-NNN: <Title>
- **Date:** YYYY-MM-DD
- **Status:** proposed | accepted | superseded by ADR-XXX
- **Context:** What problem or force makes this decision necessary?
- **Decision:** The choice made, stated as a fact.
- **Consequences:** What becomes easier, what becomes harder, what we're betting on.
- **Alternatives considered:** Each with the one-line reason it lost.
```

---

## ADR-001: Rust as the implementation language
- **Date:** 2026-07-14
- **Status:** accepted
- **Context:** The owner wants a native app with CPU parallelism; candidates were C++ and Rust.
- **Decision:** Rust, stable toolchain, edition 2024, cargo workspace.
- **Consequences:** Fearless data-parallelism via rayon; single-command builds on Windows;
  wgpu is first-class. Cost: wgpu/winit API churn between versions — pin versions per milestone.
- **Alternatives considered:** C++20/23 — more setup (CMake, dependency management), no safety
  net for the heavily threaded pipeline.

## ADR-002: CPU for data, GPU for pixels
- **Date:** 2026-07-14
- **Status:** accepted
- **Context:** "Parallel computing on CPU" is a project goal; rendering needs 60 fps with 10k aircraft.
- **Decision:** All simulation, interpolation, projection batching, and spatial indexing run
  on CPU worker threads (rayon + crossbeam channels). The GPU receives a finished, flat
  instance buffer per frame and only rasterizes.
- **Consequences:** Renderer stays trivially simple and portable (integrated GPUs fine);
  compute is debuggable/testable as plain Rust. Cost: instance-buffer upload each frame —
  bounded (~10k × 32 B ≈ 320 KB/frame, negligible).
- **Alternatives considered:** GPU compute for interpolation — needless complexity at this
  scale; full CPU software rasterizer — a fun project but not this one (revisit post-v1).

## ADR-003: wgpu + winit for windowing and graphics
- **Date:** 2026-07-14
- **Status:** accepted
- **Context:** Need a cross-backend GPU abstraction on Windows (DX12/Vulkan) with a path to
  other platforms.
- **Decision:** `winit` window/event loop, `wgpu` device/queue/pipelines, WGSL shaders.
- **Consequences:** Modern API, good validation layers, portable. Cost: version churn
  (mitigation: pin exact versions, upgrade only at milestone boundaries with a decision-log entry).
- **Alternatives considered:** raw OpenGL (glow) — simpler but legacy; egui-only — inadequate
  for custom instanced map rendering (egui may still be used later for the settings UI).

## ADR-004: SQLite (rusqlite) for persistence
- **Date:** 2026-07-14
- **Status:** accepted
- **Context:** Need durable storage for enrichment data (airports, registry), position
  history (M5), and app state — zero-administration, single file.
- **Decision:** SQLite via `rusqlite` with the `bundled` feature; schema in
  [08_DATABASE_SCHEMA.md](08_DATABASE_SCHEMA.md); embedded numbered migrations.
- **Consequences:** No server, trivially backed up, fast enough for position history with
  WAL mode + batched inserts. Cost: single-writer — fine, one process owns the DB.
- **Alternatives considered:** flat Parquet/JSON files — poor for point queries and pruning;
  sled/redb — weaker tooling for ad-hoc inspection.

## ADR-005: tokio for I/O only, rayon for compute
- **Date:** 2026-07-14
- **Status:** accepted
- **Context:** HTTP polling is async-shaped; the compute pipeline is data-parallel. Mixing
  paradigms carelessly causes starvation bugs.
- **Decision:** A small tokio runtime owns all network I/O (pollers, token refresh). Results
  cross into the sync pipeline via `crossbeam-channel`. Compute stages use rayon exclusively.
  The render thread is plain OS-thread + winit event loop.
- **Consequences:** Clear ownership boundaries; no async in core/render crates at all.
  Cost: one channel hop per poll cycle — irrelevant at 0.1–1 Hz poll rates.
- **Alternatives considered:** all-tokio (async render loop) — fights winit; all-sync with
  ureq — workable, but token refresh + concurrent multi-source polling is nicer in tokio.
