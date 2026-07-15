# Next Actions

Ordered. Top item is what the next session (or the owner) does. Post-v1 ideas go to the
parking lot — not into milestones.

## Queue

1. **[OWNER] Create OpenSky account + API client** — free, at https://opensky-network.org
   (register → account settings → create API client → note client id/secret; they'll go into
   the gitignored `config.toml` created in M0 item 0.5). Needed before M1 item 1.3; everything
   else proceeds without it. *This is the only signup the whole project requires.*
2. **Start M0** — checklist item 0.1 in [M0_REPO_AUDIT_AND_ARCHITECTURE.md](M0_REPO_AUDIT_AND_ARCHITECTURE.md)
   (cargo workspace + five crates). Use the master prompt; Opus-class session is fine
   (docs/12 mapping).
3. M0 items 0.2 → 0.8 in order, one per session.
4. M0 gate review (Fable-class session, docs/12 gate prompt), then open M1.

## Parking lot (post-v1 candidates — do not schedule)

- Own ADS-B receiver ingest (RTL-SDR / readsb local feed) — would remove API dependence for
  local traffic; revisit after v1 (risk R7).
- Light theme refinements beyond M6 baseline; OLED-black theme.
- macOS/Linux packaging.
- Day/night terminator with twilight bands (basic version is M6).
- egui-based airport detail panel (charts of METAR history).
