# 05 — Token Management for AI Sessions

Context is a recurring request cost, not free storage. This project optimizes for a small
working set that can resume from a concise handoff. The operational recovery workflow is in
[token-managed-implementation](../.claude/skills/token-managed-implementation/SKILL.md); it is
not a routine startup dependency.

## Measured hot spots

The 2026-07-20 usage report showed four concrete costs:

- **80% of usage occurred above 150k context.** Long-lived sessions are the primary cost.
- **42% came from subagent-heavy sessions.** Every agent creates another request stack.
- **14% came from renderer-agent descendants.** Nested or repeated renderer review is a
  specific fan-out problem.
- **24% came from the token-managed-implementation skill.** A skill intended to save context
  had become recurring context itself.

Treat these as regression metrics. Recheck them after several milestones; do not add process
unless it lowers one of them.

## Startup budget

Start with a bounded working set:

1. Read only `plans/CURRENT_STATUS.md`'s `Now` section; stop at the next `##` heading.
2. Read the active plan's goal/constraints and the current delivery slice only.
3. Read only the cited sections required by that slice.
4. Locate code symbols before reading source.

Never load the status session log, the whole decision log, every completed checklist note, or
the entire docs tree for orientation. `CURRENT_STATUS` is a handoff card, not project history.

Repository limits:

- `CURRENT_STATUS` Now: at most 10 bullets.
- `CURRENT_STATUS` session log: one line per session and only the 10 newest lines.
- Completed plan annotations: at most 3 lines plus a link to the decision entry.
- Files over 400 lines: targeted symbol/section reads only.
- Command output: request counts, summaries, filtered failures, or a bounded tail.

Use `rg -n` to find a decision heading and read that section only. Never read
`plans/DECISION_LOG.md` wholesale; it is an append-only archive.

## Source and tool output

- Never paste raw API responses. Trim to at most 20 representative records in a fixture and
  inspect one offending record when debugging.
- Do not read `target/`, `Cargo.lock`, generated assets, bulk CSV/GeoJSON, or fixture bodies.
- Avoid unbounded recursive listings and full build logs. Suppress successful noise and keep
  the final error plus enough surrounding context to act.
- Do not echo an applied diff into chat. For review, inspect the diff once and then only the
  affected functions where surrounding context matters.
- After a subagent returns, review its diff and affected symbols. Do not reread every changed
  file in full merely to duplicate the agent's orientation work.

## Implementation and verification

- Prefer one coherent, end-to-end delivery slice per session. It may cover adjacent low-risk
  checklist items when they share files, context, and one acceptance check. Split only when the
  work has a real design, ownership, or verification boundary.
- Plan each file's edit once. Avoid repetitive edit/compile/fix loops by reviewing ownership,
  error paths, and common Clippy lints before the first check.
- Use targeted crate tests during implementation only when they answer a specific question.
  Apply the risk tiers in doc 06 after a local diff review. Run the required final tier once;
  rerun only the failed stage after a fix. Do not run the full workspace suite merely because a
  Markdown file, prompt, or isolated leaf crate changed.
- Record the final result, not every command transcript or incremental test-count delta.
- For visible work, run automated/headless checks first and perform one focused live pass. If
  scripted navigation cannot reach the state, record a harness gap after one attempt and move on.

## Delegation budget

Default to zero subagents. Spawn one only when all of these are true:

1. The work is bounded to one lane with exact file paths and an acceptance check.
2. Its cold-start reading replaces substantial reading in the parent session.
3. The answer will be consumed as a patch or compact finding, not a second narrative audit.

Normal cap: one subagent per checklist item. A second is justified only for independent files
and independent checks. Subagents must not spawn subagents; cross-lane work returns to the
parent as a boundary. Never spawn an agent to 'double-check', independently re-derive test
counts, or reread work the parent can inspect in one diff.

For mechanical work, select the cheapest model that can follow the existing specification.
Keep judgment-heavy renderer debugging in the main session; use renderer-agent only for a
bounded implementation that does not require another agent beneath it.

## Handoff and session lifecycle

- Replace the Now section with done / next / blockers. Do not prepend forever.
- Add at most one short session-log line. Put non-trivial rationale once in `DECISION_LOG`; do
  not duplicate it in status and the milestone plan.
- Use `/compact` at a verified checkpoint if the same item must continue.
- Use `/clear` before a new checklist item, milestone, or unrelated bugfix. If reorientation
  would start by reading Now again, the old context should not come along.
- Invoke the token-managed skill only for recovery, explicit budget auditing, or a session that
  is already ballooning. Normal sessions follow the concise loop in `CLAUDE.md` directly.
