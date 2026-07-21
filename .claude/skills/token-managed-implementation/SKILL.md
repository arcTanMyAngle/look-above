---
name: token-managed-implementation
description: Recovery and budget-audit workflow for a Look Above session that is ballooning, compacted, or explicitly being audited. Normal implementation sessions follow CLAUDE.md directly and should not load this skill.
model: haiku
---

# Token-Managed Recovery

Use this skill only when the parent says context is ballooning, a compaction/crash needs
recovery, or the user requests a token audit. The full policy is
`docs/05_TOKEN_MANAGEMENT.md`; do not reread that file unless auditing the policy itself.

## Recover the working set

1. Read only `plans/CURRENT_STATUS.md` from `## Now` to the next `##`.
2. Read the active plan's goal/constraints and current unchecked item only.
3. Inspect `git diff --stat`, then targeted diff hunks for the files in that item.
4. Read only unresolved symbols or the last relevant error. Never reconstruct the old chat.

## Decide

- Same item and a verified checkpoint exists: `/compact`, then continue.
- Different item or task: write the handoff, then `/clear`.
- Item is too large: split it explicitly in the plan and finish one piece.
- Need a subagent: default no; at most one bounded, non-nesting agent with exact files and a
  check. Use the cheapest capable model for mechanical work.

## Handoff

- Replace Now with at most 10 bullets: done, next, blockers.
- Add one short session-log line; keep only the 10 newest.
- Put non-trivial rationale once in `DECISION_LOG`; do not repeat implementation narratives.
- Run the required verification once after reviewing the final diff, then stop.
