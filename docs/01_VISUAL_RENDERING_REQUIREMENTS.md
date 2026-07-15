# 01 — Visual Rendering Requirements

The renderer is a thin **wgpu** layer fed by the CPU pipeline. Its job is presentation only:
no simulation, no I/O, no allocation-heavy work inside the frame loop.

## Performance budget

| Metric | Requirement |
|---|---|
| Frame rate | 60 fps sustained; never below 30 fps with 10,000 aircraft in view |
| Frame CPU time (render thread) | ≤ 4 ms; everything else happens on worker threads |
| GPU target | Must run on integrated GPUs (Intel Iris Xe class) via wgpu's DX12/Vulkan backends |
| Startup to first map | ≤ 2 s (map geometry from bundled data, aircraft appear as feeds arrive) |
| Memory (render assets) | ≤ 256 MB |

## Pipeline shape

- One render thread owning the `wgpu::Surface`. Per frame it consumes a **render buffer** —
  a flat, pre-sorted array of instance data (position, heading, altitude-bucket, category,
  selection state) produced by the CPU interpolation stage. Double-buffered; the render
  thread never waits on the pipeline.
- Draw order: map base → map lines (coastlines/borders/runways) → trails → aircraft glyphs
  → labels → UI overlay.
- Aircraft are **instanced quads** with an SDF glyph atlas (per aircraft category: jet,
  turboprop, light, helicopter, glider, unknown). Rotation applied in the vertex shader from
  per-instance heading.
- Text via a glyph atlas (`glyphon` or equivalent); label layout (collision handled on CPU).

## Smooth motion (the core requirement)

Feeds update every 5–60 s. Aircraft must **glide**:

- Between updates, positions are dead-reckoned on CPU from last known position, ground speed,
  heading, and vertical rate (math specified in the high-fidelity-flight-visualization skill).
- When a fresh update arrives, correct the error over 1–2 s with an ease-out blend —
  never snap, never rubber-band backwards along track.
- Stale aircraft (no update > 60 s) fade over 5 s, then drop from the buffer.
- Interpolation runs at render cadence but on worker threads (rayon over the aircraft array),
  writing the next render buffer.

## View modes & level of detail

Continuous zoom, three LOD tiers with hysteresis (switch thresholds differ by ~10% between
zoom-in and zoom-out to avoid flicker):

| Tier | Zoom (approx viewport width) | Aircraft representation | Extras |
|---|---|---|---|
| **L0 Global** | > 3,000 km | Density: additive-blended dots, brightness ∝ local count | Day/night terminator optional (M6) |
| **L1 Continental** | 300–3,000 km | Small oriented glyphs, no trails | Major airports as dots |
| **L2 Regional** | < 300 km | Full glyphs, altitude-colored trails, labels, selection | Airports with runway outlines, METAR badge |

- **Projection:** L2/L1 use Web Mercator locally; L0 renders an orthographic globe. The
  transition between globe and mercator camera is animated (~400 ms) and interruptible.
- Trails: last 5 min of positions per aircraft (L2 only), tapering width and alpha, colored
  by the altitude ramp.
- Labels (L2): callsign + altitude (FL) + speed. CPU collision culling — labels never overlap;
  priority = selected > moving fast > closest to viewport center.

## Color & theme

- Dark theme default (light theme M6). Map is desaturated; **aircraft are the brightest
  things on screen.**
- Altitude ramp (trails + optional glyph tint): ground/taxi gray → low warm amber → mid
  yellow-green → cruise cyan → high (FL400+) violet. Perceptually ordered, colorblind-checked
  (distinguishable under deuteranopia; ramp encodes lightness monotonically, not hue alone).
- Selection: white outline + info card. Emergency squawks (7500/7600/7700): pulsing red ring —
  display only, no alerting features.

## Quality bar

- MSAA 4x (or SDF-derived AA) — no shimmering glyph edges.
- No visible teleporting, popping, or label flicker during pan/zoom or LOD transitions.
- Every visual claim above is testable; the checklist form lives in
  [13_VISUAL_QA_CHECKLIST.md](13_VISUAL_QA_CHECKLIST.md), renderer smoke tests in
  [10_TEST_PLAN.md](10_TEST_PLAN.md).
