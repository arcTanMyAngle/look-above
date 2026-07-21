# 13 — Visual QA Checklist

Run against the live app (`cargo run --release -p look-above`). Full pass required at M4+
gates; the **L2-core** subset at the M2 gate. Compare every item against
[01_VISUAL_RENDERING_REQUIREMENTS.md](01_VISUAL_RENDERING_REQUIREMENTS.md) budgets. Record
results (pass/fail + notes + frame-stats numbers) in `plans/CURRENT_STATUS.md`.

Setup: busy region (e.g. bbox around a major hub), ≥ 200 live aircraft, frame-stats overlay
on, release build, laptop on mains power.

## Efficient execution

For an individual visible feature, run only the relevant subsection once after headless/unit
checks pass. A full checklist pass is a milestone-gate activity. Prefer deterministic camera
presets or test scenes over synthetic mouse automation. If the required view cannot be reached
or captured after one focused attempt, record that harness gap and stop; do not turn feature
verification into an open-ended window-control debugging session.

## L2-core (regional mode) — required at M2

**Motion**
- [ ] Watch one aircraft for 3 minutes: continuous glide, no teleports, no backwards jumps when updates arrive.
- [ ] Aircraft in a turn (find one on approach): trail curves smoothly, glyph heading leads the turn plausibly.
- [ ] Kill the network for 90 s: aircraft keep dead-reckoning, then fade out by ~65 s of staleness; restore network: reacquired aircraft blend in, no snap.
- [ ] Frame stats: p95 frame time < 16.6 ms during continuous pan; no hitching when a poll cycle lands.

**Glyphs & trails**
- [ ] Glyph orientation matches reported true track for 10 spot-checked aircraft (click → compare card value vs on-screen heading).
- [ ] Trails taper (width + alpha), are continuous (no gaps/kinks), max ~5 min length.
- [ ] Altitude color ramp reads correctly: taxiing gray, departing climbs through amber→green, cruise cyan, FL400+ violet.
- [ ] Glyph edges clean at all zooms — no shimmer while panning (AA working).

**Labels**
- [ ] Zero overlapping labels at any moment during a 2-minute observation of a busy terminal area.
- [ ] Selected aircraft's label always visible; label priority sensible (selected > fast > central).
- [ ] Labels don't flicker in/out at a fixed zoom; culling changes only on real state change.

**Map**
- [ ] Coastlines/borders crisp, desaturated; aircraft are visually dominant over the map.
- [ ] Pan/zoom inertia feels right; no seams, cracks, or missing polygons in the viewport.

## L1/L0 + transitions — required at M4

- [ ] Zoom out from runway to globe in one continuous gesture: L2→L1 at ~300 km viewport (trails+labels drop), L1→L0 at ~3,000 km (glyphs→density) — transitions cross-fade, nothing pops.
- [ ] Dither zoom ±5% around each threshold: tier does not flip back and forth (hysteresis).
- [ ] Globe↔mercator camera animation ≤ 500 ms, interruptible; no horizon clipping artifacts.
- [ ] L0 density honestly reflects traffic (North Atlantic band at night, US/Europe density contrast); no fake per-plane glyphs at global scale.
- [ ] 8,000+ aircraft in global view: p95 frame time < 16.6 ms.

## Selection & overlays — required at M3/M4

- [ ] Click hit-testing accurate at all zooms (glyph, not label, is the target).
- [ ] Info card: normal aircraft shows callsign/type/operator/route or "—"; anonymous aircraft shows "Unidentified" with position data only and no route (privacy 2.2).
- [ ] Emergency squawk styling (pulsing ring) visible but not alarming; disappears when squawk clears.
- [ ] METAR badge colors match flight category (VFR green / MVFR blue / IFR red / LIFR magenta) for 3 spot-checked airports vs aviationweather.gov.

## Accessibility & theme

- [ ] Altitude ramp distinguishable in a deuteranopia simulation (lightness ordering survives).
- [ ] All UI text ≥ 4.5:1 contrast against its background (both themes at M6).
- [ ] UI readable at 125% and 150% Windows display scaling.

## Evidence

Attach (or reference paths of) screenshots for: busy L2 view, label-dense area, L0 globe,
one anonymous-aircraft card. Store under `qa/<date>/` (gitignored).
