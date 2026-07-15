# 00 — Product Vision

## One-liner

**Look Above** is a native Rust flight tracker that makes live air traffic feel tangible —
smooth, dense, and beautiful — using only free, authorized data, on hardware you already own.

## Why it exists

Web trackers (FlightRadar24, FlightAware) are excellent but are ad-driven browser apps with
choppy update rates on free tiers, and their data may not be reused programmatically. Look
Above is the opposite trade: a personal, native tool where the craft is in the *pipeline* —
CPU-parallel ingestion, physically sensible interpolation between sparse API updates, and a
GPU presentation layer that stays at 60 fps whether you're looking at the whole planet or one
approach corridor.

## Who it's for

- The project owner: a developer who wants a high-quality systems project (Rust, parallelism,
  graphics) that produces something genuinely fun to leave running on a second monitor.
- Secondarily: aviation hobbyists who want a free, private, no-account-required (degraded
  mode) desktop tracker.

## Product pillars

1. **Fluid motion from sparse data.** Free APIs update every 5–60 s. The app dead-reckons
   between updates so aircraft glide rather than teleport. This is the core technical bet and
   the main quality differentiator.
2. **Two honest zoom modes.** A *global* mode that shows traffic density truthfully (no fake
   per-plane detail at planetary scale) and a *regional* mode with oriented glyphs,
   altitude-colored trails, and readable labels. LOD transitions are continuous, not a jarring
   mode switch.
3. **CPU for data, GPU for pixels.** All simulation, projection math, and spatial queries are
   CPU-parallel (rayon); the GPU does instanced drawing only. The app must run well on
   integrated graphics.
4. **Free and legitimate.** Every byte of data comes from a source that permits programmatic
   use at zero cost. Rate limits are budgeted, not dodged. Attribution is displayed.
5. **Privacy by design.** Aircraft in blocking programs (LADD/PIA) are never identified.
   We show what the authorized feeds show, nothing more.

## Non-goals

- Not a FlightRadar24 clone or competitor; no web version, no mobile, no sharing features.
- No paid data sources, ever. No scraping of sites that prohibit it, ever.
- No own-hardware ADS-B receiver support in v1 (possible later; see risk register).
- No historical "who flew where" investigations — history features (M5) are for replaying
  what *you* watched, with the same privacy rules applied.
- No flight *prediction* / ETA products; dead reckoning is a rendering technique here, not a
  navigation claim.

## Success criteria (v1 = end of M6)

- Runs on the owner's Windows 11 machine at 60 fps with 8,000+ live aircraft in global mode.
- Regional mode over a busy TRACON looks *right*: correct headings, smooth motion, no label
  soup, trails that tell the traffic story at a glance.
- Zero-cost operation indefinitely within OpenSky's free registered tier + no-key fallbacks.
- A stranger could clone the repo, add OpenSky credentials, and be watching traffic in
  under five minutes.
