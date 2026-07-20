# 05 — Token Management for AI Sessions

Context (tokens) is the scarcest resource in an AI implementation session. These rules keep
each session cheap, focused, and resumable. The workflow that applies them end-to-end is the
[token-managed-implementation skill](../.claude/skills/token-managed-implementation/SKILL.md).

## Reading budget

- **Always read (small, load-bearing):** `plans/CURRENT_STATUS.md`, the active milestone
  plan, `CLAUDE.md` (auto-loaded).
- **Read on demand only:** the specific docs the current checklist item cites. A renderer
  item needs docs 01 + 13; an ingestion item needs the sources skill + doc 09. Nothing else.
- **Never read wholesale:** the entire `docs/` tree, generated files, lockfiles,
  `target/`, fixture bodies (read fixture *names* and one sample record at most).
- Prefer targeted `Grep`/section reads over whole-file reads for files > ~200 lines.

## API responses and fixtures

- **Never paste a raw API response into a session.** A single OpenSky `/states/all` global
  snapshot is megabytes. Instead: fetch → trim to ≤ 20 representative records → save to
  `tests/fixtures/<source>/<case>.json` → reference by path.
- When debugging a parse failure, extract the *one* offending record into the session, not
  the payload.

## Writing budget

- One checklist item per session. If an item is ballooning, split it in the plan file and
  finish the first piece properly rather than half-finishing everything.
- Don't restate file contents in chat; reference paths. Don't echo diffs already applied.
- Summaries in CURRENT_STATUS are ≤ 10 lines: done / next / blockers / decisions-logged.

## Delegation (subagents)

- Delegate when a subtask is (a) in one agent's lane and (b) would require reading files the
  main session otherwise doesn't need — e.g., "write the WGSL shader for trail tapering"
  goes to renderer-agent with docs 01 §pipeline as its only context.
- Give subagents explicit file paths and the acceptance criterion. They start cold; a vague
  prompt wastes their whole budget re-exploring.
- Don't delegate trivial edits (< ~20 lines in files already in context) — the handoff costs
  more than the work.
- **Each subagent spawn is its own request stack, not a discount.** Don't spawn one to
  "double-check" or "also look at" something the main session could answer with a single
  targeted `Grep`/`Read`. Before spawning, name the specific files-you'd-otherwise-read that
  the delegation avoids; if you can't name any, do it inline instead.
- Prefer one well-scoped agent over several overlapping ones for the same checklist item.
  Fan-out (parallel agents on independent sub-parts) is fine; fan-out on the *same* question
  from multiple angles usually isn't.
- For mechanical/lane-bounded subtasks (fixture trimming, formatting a doc, running a fixed
  checklist), pick the cheapest model that can do the job — see the model-to-task mapping
  below. Don't default every subagent to the same model as the main session.

## Usage data & session hygiene

Observed from this project's actual usage (not theoretical): **64% of token usage happened
in sessions that had grown past 150k context**, and **63% of usage came from
subagent-heavy sessions**. Both are addressable with habits, not just budget rules:

- **Context size compounds cost, even with caching.** A session sitting above ~150k tokens
  of context is expensive on every subsequent turn, cached or not. Don't let a session ride
  past that just because it's "almost done" — `/compact` at a natural pause point *inside* a
  task (after verify, before the next checklist item) rather than carrying full history to
  the end.
- **`/clear` between unrelated tasks, always.** Starting a new checklist item, milestone, or
  unrelated bugfix in a session that still holds a prior task's full context is the single
  biggest avoidable cost. If the next thing you'd read is `CURRENT_STATUS.md` again to
  re-orient, that's the signal to `/clear` first, not to keep scrolling back.
  Session lifecycle in the skill (§5–6) already ends sessions at handoff boundaries — treat
  that as the trigger to `/clear`, not just to stop typing.
  Deferred to the skill: [token-managed-implementation](../.claude/skills/token-managed-implementation/SKILL.md).
- **Subagent spawns are the second-biggest lever.** Every spawn re-derives context from
  scratch at its own cost; three "quick check" subagents in one session can outspend doing
  the checks inline. Re-read the delegation rules above before reaching for `Agent` out of
  habit.

## Session lifecycle

- **Handoff, not marathon.** End the session at the natural boundary (checklist item done,
  verified, status updated). The next session resumes from CURRENT_STATUS with near-zero
  re-orientation cost.
- If a session gets compacted mid-task, CURRENT_STATUS + the milestone checklist are the
  recovery points — which is why they're updated *before* risky/long operations, not only at
  the end.
- Model-to-task mapping (use cheaper models for mechanical work):
  [12_PROMPTS_FOR_CLAUDE_OPUS_SONNET_FABLE.md](12_PROMPTS_FOR_CLAUDE_OPUS_SONNET_FABLE.md).
