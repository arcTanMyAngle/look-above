# Next Actions

Ordered. Top item is what the next session (or the owner) does. Post-v1 ideas go to the
parking lot — not into milestones.

## Queue

1. **[OWNER] Rename the repo, then push — in that order.** `origin` is already set to
   `git@github.com:arcTanMyAngle/look-above.git` (2026-07-15). Two steps, and the order
   matters — the remote points at the *hyphenated* name, which does not exist yet:

   1. **Rename on GitHub:** repo → Settings → rename `look_above` → **`look-above`**. The
      hyphen is the canonical identity (DECISION_LOG 2026-07-15): it's in the User-Agent we
      send every aviation source (docs/09) and in the README badge, and both 404 until this
      lands. GitHub redirects the old URL, so nothing breaks.
   2. **Push:** `git push -u origin main`. Not doable from a Claude session — this machine has
      no SSH key (`~/.ssh` has only `known_hosts`; `git@github.com` → `Permission denied
      (publickey)`). Push from your own terminal.

   **This is the last unmet M0 acceptance line:** the 0.8 gate met 6 of 7; "CI runs on push;
   badge green" is the seventh. M1 was opened anyway on 2026-07-15 at the owner's direction,
   so this is overdue rather than merely pending — every M1 commit lands unverified by CI.
   When the push does land, watch the **Linux** job: it has never executed (DECISION_LOG 0.7,
   "no apt step", is the first suspect if it fails).

   *Also decide:* the repo is **public**, but inception recorded "private by default until
   owner says otherwise" (DECISION_LOG 2026-07-14). Nothing sensitive is exposed — either
   flip it to private or amend the record.
2. **[OWNER] Create OpenSky account + API client** — free, at https://opensky-network.org
   (register → account settings → create API client → note client id/secret; they'll go into
   the gitignored `config.toml` created in M0 item 0.5). **Needed before M1 item 1.3, which
   is two items away** — 1.2 does not need it. *This is the only signup the whole project
   requires.*
3. **Continue M1** — [M1_AUTHORIZED_DATA_INGESTION.md](M1_AUTHORIZED_DATA_INGESTION.md),
   item 1.2 (host allowlist). 1.1 done 2026-07-15.
4. **[OWNER] Review the M0 gate record** — evidence table in
   [CURRENT_STATUS.md](CURRENT_STATUS.md). 6/7 met, the seventh is #1 above. M1 proceeding
   does not retire this; it just means the review is happening late.

## Parking lot (post-v1 candidates — do not schedule)

- Own ADS-B receiver ingest (RTL-SDR / readsb local feed) — would remove API dependence for
  local traffic; revisit after v1 (risk R7).
- Light theme refinements beyond M6 baseline; OLED-black theme.
- macOS/Linux packaging.
- Day/night terminator with twilight bands (basic version is M6).
- egui-based airport detail panel (charts of METAR history).
