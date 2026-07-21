# 06 — Tool Usage Rules (AI sessions)

How to use tools well in this repo. Token-budget companion: [05_TOKEN_MANAGEMENT.md](05_TOKEN_MANAGEMENT.md).

## Files & search

- Use dedicated tools (Read / Grep / Glob / Edit / Write) over shell equivalents
  (`cat`, `Select-String`, `Get-ChildItem -Recurse`).
- `Grep` before `Read`: locate the symbol, then read just that region.
- For files over 400 lines, never use an unbounded `Read`; inspect the diff and the affected symbols with bounded ranges.
- Never read `target/`, `Cargo.lock`, or fixture bodies into context.
- Cap tool output. Prefer counts, `--quiet`, test filters, and the last relevant error over full logs or recursive listings.
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
cargo test -p <crate>                       # isolated-crate final check
cargo clippy -p <crate> --all-targets -- -D warnings
cargo fmt --check
cargo test --workspace                      # high-risk/cross-cutting final check
cargo clippy --workspace --all-targets -- -D warnings
cargo run --release -p look-above           # only when visible behavior changed
cargo bench -p core                          # only when a perf item asks for it
```

### Verification tiers

| Change | Required final verification |
|---|---|
| Docs, plans, prompts, agent/skill instructions only | `git diff --check`; verify edited links/paths |
| Isolated implementation inside one leaf crate | `cargo fmt --check`, crate Clippy, crate tests |
| Public contracts, cross-crate wiring, privacy, HTTP, migrations, concurrency, renderer/GPU | Full workspace fmt, Clippy, and tests |
| Milestone gate or release candidate | Full workspace sequence plus the applicable acceptance/visual checks |

Escalate to a higher tier when uncertain. Run the chosen final tier once after reviewing the
diff; during implementation, use only checks that answer a specific question.

- Long builds: run in background, keep working on something read-only meanwhile.
- Tests are offline by construction — if a test needs the network, it's wrong; use fixtures.
- Run live visual QA only when pixels or interaction should change. Exercise the relevant
  subsection once after automated checks; full checklist passes belong at gates.
- Prefer a deterministic camera/test preset. If synthetic input or window capture fails once,
  record the missing harness capability and stop rather than repeatedly debugging the QA tool.

## Subagents

- Match the lane (see [../AGENTS.md](../AGENTS.md) roster). Prompt with: the goal, exact file
  paths to touch, bounded doc sections, and the acceptance check.
- Default to zero agents and cap normal work at one. Parallel agents require independent files
  and checks; subagents must not spawn subagents.
- Review returned work from its diff and affected symbols. Re-running fixed checks is useful;
  rereading every changed file in full or asking another agent for a generic audit is not.

## Git

- Commit per completed checklist item; message format:
  `M<milestone>: <what> (<rule/doc refs if privacy-relevant>)`.
- Never commit: `config.toml`, `credentials.json`, credentials in any other form, `target/`,
  untrimmed fixtures, screenshots. (`.gitignore` enforces; don't fight it.)
- Don't push, tag, or touch remotes unless the session was asked to.
