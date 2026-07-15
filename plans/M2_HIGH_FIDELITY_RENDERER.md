# M2 — High-Fidelity Renderer

**Goal:** live regional traffic drawn at 60 fps with smooth interpolated motion — the moment
the project becomes *watchable*. Exit criteria: [../docs/11_ACCEPTANCE_CRITERIA.md](../docs/11_ACCEPTANCE_CRITERIA.md) §M2.
Constraining docs: 01 (all budgets), 09 (`RenderFeed`), 10 (§4 smoke tests), 13 (§L2-core),
and the [high-fidelity-flight-visualization skill](../.claude/skills/high-fidelity-flight-visualization/SKILL.md).

## Checklist

- [ ] 2.1 `render::gpu`: device/queue/surface init (prefer DX12 on Windows, fall back per
      wgpu defaults), swapchain resize, MSAA 4x target, frame-stats overlay (p50/p95 frame
      time, instance counts) toggled with F3.
- [ ] 2.2 Base map: Natural Earth 1:50m land + coastlines bundled as GeoJSON; tessellate once
      at startup (`lyon`) into static vertex buffers; line + fill pipelines; desaturated dark
      palette per docs/01.
- [ ] 2.3 Camera (regional): Web Mercator, pan (drag) + zoom (wheel, cursor-anchored) with
      inertia; viewport→bbox exposed to the poller (M1 poller re-targets on camera settle,
      debounced 2 s).
- [ ] 2.4 `core::sim`: interpolation/dead-reckoning worker — rayon over the live aircraft
      table at render cadence; destination-point advance from last fix (speed/track/vert
      rate); ease-out correction blend (≤ 2 s) on new fix; stale fade (60 s + 5 s); writes
      `RenderFeed` into the double buffer. Unit tests per docs/10 §1.
- [ ] 2.5 Aircraft glyphs: SDF atlas (6 categories per docs/01), instanced quad pipeline,
      per-instance rotation from heading, altitude-bucket tint attribute (final ramp colors
      may land in M4; buckets wired now).
- [ ] 2.6 Trails (in-memory, last 5 min): per-aircraft ring buffer → triangle-strip ribbons,
      taper width/alpha, altitude-ramp coloring.
- [ ] 2.7 Labels: glyph-atlas text (callsign + FL + kt), CPU collision culling with priority
      (docs/01), leader-line when displaced.
- [ ] 2.8 Selection: cursor hit-test against glyph quads (CPU, spatial index), white outline,
      minimal info card (callsign/alt/speed/source — enrichment fields arrive in M3;
      anonymous → "Unidentified" already enforced here).
- [ ] 2.9 Renderer smoke test (headless, per docs/10 §4) wired into CI (skip-if-no-adapter).
- [ ] 2.10 Gate: live run over a busy hub; visual QA §L2-core; frame-stats evidence; human review.

## Design notes

- Render thread does: swap buffer, upload instances, record command buffer, present. Nothing
  else. All layout/culling/interpolation happens on workers (ADR-002).
- Instance buffer uploads use a persistently mapped staging ring; budget in docs/01 makes
  this comfortably small — don't over-engineer.
- Spatial index (for hit-testing and later label density): start with a simple uniform grid
  over screen space rebuilt per frame; r-tree only if profiling demands it (record either
  way in DECISION_LOG).
- The interpolation math is specified in the visualization skill — implement to that spec so
  tests and code cite the same formulas.
