---
name: high-fidelity-flight-visualization
description: The math and specs that make Look Above's rendering feel alive - dead-reckoning interpolation between sparse API updates, correction blending, stale fades, LOD tier specs, glyph/trail/label rendering rules, and the altitude color ramp. Consult before any work in core::sim or crates/render, or when motion looks wrong.
---

# High-Fidelity Flight Visualization

The core bet of this project: free feeds update every 5–60 s, yet aircraft must **glide**.
This skill specifies the math so `core::sim`, its tests, and the renderer all agree.
Budgets and requirements: `docs/01_VISUAL_RENDERING_REQUIREMENTS.md`.

## Dead reckoning (between updates)

Each aircraft holds its last fix: position **p₀** (lat/lon), ground speed *v* (m/s), true
track *θ*, vertical rate *ṙ* (m/s), at source time *t₀*. At render time *t*, with
Δt = t − t₀ (clamped to ≤ 90 s):

- Distance along track: d = v·Δt
- Position: destination-point on the sphere (R = 6 371 008.8 m):
  - φ₂ = asin(sin φ₁·cos(d/R) + cos φ₁·sin(d/R)·cos θ)
  - λ₂ = λ₁ + atan2(sin θ·sin(d/R)·cos φ₁, cos(d/R) − sin φ₁·sin φ₂)
- Altitude: h = h₀ + ṙ·Δt (clamp ≥ 0)
- Missing fields: no speed → hold position; no track → hold position, keep last heading for
  the glyph; `on_ground` → never extrapolate (taxi motion from data only).
- Track is held constant between fixes (no turn-rate estimation in v1 — heading *changes*
  arrive with the next fix and are handled by the blend below).

## Correction blend (when a new fix arrives)

Never snap, never rubber-band. On fix arrival at time t₁:

1. Let **p_shown** = currently displayed (dead-reckoned) position; **p_new** = the fix,
   itself dead-reckoned forward from its source timestamp to *now* (fixes are already stale
   on arrival).
2. Over blend window w = min(2 s, time-to-next-expected-fix/2): display
   p(t) = slerp(p_shown, p_new_extrapolated(t), ease_out(u)) where u = (t−t₁)/w,
   ease_out(u) = 1−(1−u)², and p_new keeps dead-reckoning during the blend (chase a moving
   target, or you re-snap at u=1).
3. Heading/altitude blend the same way (shortest-arc for heading).
4. **Invariant (tested):** the displayed position never moves backwards along the track
   direction. If the correction would require it (new fix behind shown position), slow the
   shown aircraft (blend speed toward the fix's) instead of reversing.
5. Teleport exception: if error > 10 km (data gap, wrong aircraft merge), snap with a 300 ms
   fade-out/in rather than a visible slide across the map.

## Staleness

- Δt > 60 s: begin fade (alpha → 0 over 5 s), stop extrapolating at 90 s (frozen while
  fading). Remove from RenderFeed after fade. Reacquisition uses the correction blend,
  or the teleport exception if it moved far.

## LOD tiers (with hysteresis)

| Tier | Enter (zoom out) | Enter (zoom in) | Aircraft | Trails | Labels |
|---|---|---|---|---|---|
| L0 global | viewport > 3,300 km | — | additive density dots (2 px), brightness ∝ local count | no | no |
| L1 continental | > 330 km | < 3,000 km | glyphs 8–12 px, heading-rotated | no | no |
| L2 regional | — | < 300 km | glyphs 16–24 px | 5 min, tapered | yes |

Cross-fade tiers over 250 ms; the ~10% threshold gap is the hysteresis that prevents
flip-flicker. Glyph size interpolates within a tier (zoom-proportional, clamped).

## Glyphs

SDF atlas, 6 categories: jet (swept), turboprop (straight wing), piston/light (high wing),
helicopter (rotor disc), glider (long wing), unknown (simple dart). Category from
`aircraft.category` (feed or adsbdb), default unknown. Rotation = smoothed display heading.
Selected: white outline (2 px). Emergency squawks 7500/7600/7700: pulsing red ring, 1 Hz,
passive only (privacy rule 6.1).

## Altitude color ramp (trails + optional glyph tint)

Perceptually ordered, lightness-monotonic (colorblind-safe — verify in deuteranopia sim):

| Altitude | Color | Hex |
|---|---|---|
| ground/taxi | gray | `#6E7076` |
| < 2,000 ft | warm amber | `#C97B3D` |
| 2,000–10,000 ft | yellow-green | `#A8B84B` |
| 10,000–28,000 ft | green-cyan | `#4DBE8F` |
| 28,000–40,000 ft | cyan | `#3FA9D0` |
| > FL400 | violet | `#8B7BD8` |

Interpolate in a perceptual space (Oklab) between stops, not RGB.

## Trails

Ring buffer of the last 5 min of *displayed* positions (so trails inherit smoothness),
sampled at ≥ 1 Hz. Ribbon: width 3 px → 0.5 px, alpha 0.8 → 0, altitude-ramp colored per
vertex. L2 only.

## Labels (L2)

Content: `CALLSIGN  FL350  450kt` (omit unknowns; anonymous targets get no label unless
selected → "Unidentified"). Placement: right of glyph, flip left near viewport edge; leader
line if displaced > 24 px. Collision: CPU sweep, priority = selected > speed > proximity to
viewport center; losers culled entirely (no overlap, no shrink). Re-evaluate at ≤ 5 Hz, not
per frame, to avoid flicker; a label keeps its slot until its priority is beaten by > 10%.

## Performance recipe (ADR-002)

`sim` advances all aircraft in a rayon parallel iterator over a flat array (budget: 10k in
< 2 ms / 8 cores); projection is batched the same way; results written into the inactive
render buffer, swapped atomically at frame start. The render thread never computes any of
the above.
