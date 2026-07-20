---
name: token-managed-implementation
description: The session workflow for implementing Look Above under a token budget - what to read, how to scope to one checklist item, when to delegate to subagents, and the mandatory handoff (CURRENT_STATUS, DECISION_LOG, commit). Consult at the start of any implementation session and whenever a session is ballooning.
---

# Token-Managed Implementation

One session = one milestone checklist item, verified, with a clean handoff. Budget rules
live in `docs/05_TOKEN_MANAGEMENT.md`; this is the operational procedure.

## 1. Orient (cheap — ~3 file reads)

Read, in order, **and nothing else yet**:
1. `plans/CURRENT_STATUS.md` → the next action and any blockers.
2. The active milestone plan it names → your checklist item + its "constraining docs" list.
3. Only those constraining docs — and only the relevant sections for files > ~200 lines
   (Grep to the section, don't read whole files).

If CURRENT_STATUS names a blocker only the owner can clear (e.g., missing OpenSky
credentials), stop immediately and restate the blocker — don't work around it.

## 2. Scope

Restate the single item you'll complete. Too big for one session? Split it **in the plan
file first** (sub-items 2.4a/2.4b…), then do the first piece. Never leave splits implicit.

## 3. Implement

- Smallest correct change; conventions from `CLAUDE.md` (fmt/clippy clean, thiserror, no
  unwrap outside tests).
- Consult the lane skill *before* coding: authorized-aviation-sources for anything HTTP;
  high-fidelity-flight-visualization for sim/render work.
- **Delegate** to a `.claude/agents/` subagent when the subtask is (a) in one lane and
  (b) would force you to read files you otherwise don't need. Prompt template:

  > Task: <goal>. Files: <exact paths>. Constraints: <doc sections>. Done when: <check>.

  Don't delegate < ~20-line edits in files already in your context — handoff costs more.
- Never paste raw API payloads or fixture bodies into the session (one offending record max,
  when debugging a parse).

## 4. Verify

`cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace`.
Renderer-visible items: run the app, check the relevant `docs/13` items. Perf-budgeted
items: run the bench, report the number. **No green, no done** — a failing check is either
fixed or recorded as the exact blocker.

## 5. Hand off (mandatory, even on a blocked/failed session)

1. Tick the checklist item in the milestone plan (or annotate why not).
2. Update `plans/CURRENT_STATUS.md` Now-section (≤ 10 lines: done / next / blockers) and
   append one line to its session log.
3. Append any decision made (new dep, split item, deviation) to `plans/DECISION_LOG.md`.
4. Commit: `M<X>: <what>` (+ privacy rule refs if docs/04-relevant). Don't push unless asked.

## 6. Stop

- Item done + handoff written → end the session. Resist "while I'm here" work: it belongs
  in `plans/NEXT_ACTIONS.md` or the next session.
- Last item of a milestone → run the gate per `docs/11_ACCEPTANCE_CRITERIA.md`, record
  evidence in CURRENT_STATUS, and stop for human review.
- **Starting a different, unrelated task next → `/clear` first**, even mid-conversation.
  Usage data shows most token spend happens in sessions carried past ~150k context or
  reused across unrelated tasks — see `docs/05_TOKEN_MANAGEMENT.md` §Usage data. If a
  single session is running long but still on the *same* item, `/compact` at a verified
  checkpoint (post-step-4) instead of carrying full history forward.

## Recovery (compaction / crashed session)

CURRENT_STATUS + the milestone checklist are the recovery points — update CURRENT_STATUS
*before* long or risky operations (big refactor, live-run gate), not only at the end.
A fresh session must be able to resume from those two files alone.
