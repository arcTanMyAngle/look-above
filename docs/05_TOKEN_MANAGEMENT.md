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

## Session lifecycle

- **Handoff, not marathon.** End the session at the natural boundary (checklist item done,
  verified, status updated). The next session resumes from CURRENT_STATUS with near-zero
  re-orientation cost.
- If a session gets compacted mid-task, CURRENT_STATUS + the milestone checklist are the
  recovery points — which is why they're updated *before* risky/long operations, not only at
  the end.
- Model-to-task mapping (use cheaper models for mechanical work):
  [12_PROMPTS_FOR_CLAUDE_OPUS_SONNET_FABLE.md](12_PROMPTS_FOR_CLAUDE_OPUS_SONNET_FABLE.md).
