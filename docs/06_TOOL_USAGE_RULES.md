# 06 — Tool Usage Rules (AI sessions)

How to use tools well in this repo. Token-budget companion: [05_TOKEN_MANAGEMENT.md](05_TOKEN_MANAGEMENT.md).

## Files & search

- Use dedicated tools (Read / Grep / Glob / Edit / Write) over shell equivalents
  (`cat`, `Select-String`, `Get-ChildItem -Recurse`).
- `Grep` before `Read`: locate the symbol, then read just that region.
- Never read `target/`, `Cargo.lock`, or fixture bodies into context.
- Edits over rewrites: prefer `Edit` on existing files; `Write` only for new files.

## Network

- **No ad-hoc live API calls during implementation.** The only sanctioned live fetches are:
  (a) the fixture-recording script (`scripts/record_fixture.rs`, M1) which trims and saves
  output without displaying it, and (b) running the app itself.
- If you must inspect an endpoint's shape, fetch with output redirected to a scratch file,
  then read ≤ 20 lines of it.
- Any new HTTP host in code must already be on the allowlist
  ([authorized-aviation-sources](../.claude/skills/authorized-aviation-sources/SKILL.md)) — rule 1.1 of
  [04_PRIVACY_AND_SAFETY_RULES.md](04_PRIVACY_AND_SAFETY_RULES.md).

## Build, test, run

```sh
cargo check -p <crate>                      # fast inner-loop signal
cargo test -p <crate> <filter>              # targeted tests while iterating
cargo test --workspace                      # before claiming done
cargo clippy --workspace -- -D warnings     # before claiming done
cargo fmt                                    # before committing
cargo run --release -p look-above           # visual verification (M2+)
cargo bench -p core                          # only when a perf item asks for it
```

- Long builds: run in background, keep working on something read-only meanwhile.
- Tests are offline by construction — if a test needs the network, it's wrong; use fixtures.
- Renderer verification is visual: run the app, compare against
  [13_VISUAL_QA_CHECKLIST.md](13_VISUAL_QA_CHECKLIST.md); screenshots when the harness supports it.

## Subagents

- Match the lane (see [../AGENTS.md](../AGENTS.md) roster). Prompt with: the goal, exact file
  paths to touch, the doc sections that constrain the work, and the acceptance check.
- One agent per lane at a time; parallel agents only for independent lanes (e.g., testing-agent
  writing fixtures while renderer-agent writes a shader).

## Git

- Commit per completed checklist item; message format:
  `M<milestone>: <what> (<rule/doc refs if privacy-relevant>)`.
- Never commit: `config.toml`, credentials, `target/`, untrimmed fixtures, screenshots.
  (`.gitignore` enforces; don't fight it.)
- Don't push, tag, or touch remotes unless the session was asked to.
