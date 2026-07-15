# Next Actions

Ordered. Top item is what the next session (or the owner) does. Post-v1 ideas go to the
parking lot — not into milestones.

## Queue

1. **[OWNER] Create the GitHub remote and push** — the repo has no remote. Docs already fix
   the identity as `github.com/arcTanMyAngle/look-above` (it's in the User-Agent, docs/09),
   and README's CI badge points at that repo's `ci.yml`. Until a push happens the badge 404s
   and CI has never run, which is the one acceptance §M0 line the 0.8 gate cannot verify
   locally. Everything else in M0 is checkable offline.
2. **[OWNER] Create OpenSky account + API client** — free, at https://opensky-network.org
   (register → account settings → create API client → note client id/secret; they'll go into
   the gitignored `config.toml` created in M0 item 0.5). Needed before M1 item 1.3; everything
   else proceeds without it. *This is the only signup the whole project requires.*
3. **M0 item 0.8 — the gate.** Items 0.1–0.7 are done. Run acceptance §M0
   ([../docs/11_ACCEPTANCE_CRITERIA.md](../docs/11_ACCEPTANCE_CRITERIA.md)), record results in
   CURRENT_STATUS, stop for human review. Fable-class session (docs/12 gate prompt).
   Still unverified going in: clean-clone `cargo build --workspace`, the `cargo tree`
   dependency direction, and the CI badge (blocked on #1).
4. **Open M1** after the gate passes — [M1_AUTHORIZED_DATA_INGESTION.md](M1_AUTHORIZED_DATA_INGESTION.md).

## Parking lot (post-v1 candidates — do not schedule)

- Own ADS-B receiver ingest (RTL-SDR / readsb local feed) — would remove API dependence for
  local traffic; revisit after v1 (risk R7).
- Light theme refinements beyond M6 baseline; OLED-black theme.
- macOS/Linux packaging.
- Day/night terminator with twilight bands (basic version is M6).
- egui-based airport detail panel (charts of METAR history).
