# 12 — Prompts by Model (Fable / Opus / Sonnet)

The [master prompt](../prompts/FLIGHT_TRACKER_MASTER_PROMPT.md) is the base for every
session. This doc maps task types to models and gives ready-to-paste openers. Principle:
**spend the big model on decisions, the fast models on execution.**

## Task → model map

| Task type | Model | Why |
|---|---|---|
| Architecture, ADRs, milestone gate reviews, hard debugging (race conditions, wgpu validation errors, motion artifacts) | **Fable** | Judgment-heavy, cross-cutting, expensive to get wrong |
| Feature implementation (adapters, pipeline stages, shaders, camera), tricky refactors | **Opus** | Strong code quality per token on scoped work |
| Tests from an existing spec, fixtures, docs updates, mechanical refactors, CI wiring, CSV importers | **Sonnet** | Fast + cheap; the spec already constrains the work |

Rule of thumb: if the checklist item's plan already says *how*, Sonnet or Opus executes it;
if the session must decide *how*, Fable.

This mapping applies to both the orchestrator and any subagent. Do not let a simple subagent
inherit an expensive model by accident. Renderer-agent is for a bounded implementation on
exact files; keep cross-cutting wgpu diagnosis in the main judgment-heavy session.

## Ready-to-paste openers

Each assumes the repo is the working directory (CLAUDE.md auto-loads) and appends to the
master prompt's session loop.

### Fable — milestone gate review
```text
Follow prompts/FLIGHT_TRACKER_MASTER_PROMPT.md. Task: gate review for milestone <MX>.
Verify every item in docs/11_ACCEPTANCE_CRITERIA.md §<MX> with evidence (run the checks,
don't trust the checklist). Record pass/fail + numbers in plans/CURRENT_STATUS.md.
If it passes: draft the next milestone's plan file per docs/07 and stop for human review.
If it fails: file the gaps as unchecked items in the milestone plan and stop.
```

### Fable — hard debugging
```text
Follow prompts/FLIGHT_TRACKER_MASTER_PROMPT.md. Task: diagnose <symptom>.
Reproduce first (exact command + observation), then hypothesize and bisect. Constraints in
docs/01 (renderer) / docs/09 (contracts) apply. Write the root cause and fix rationale to
plans/DECISION_LOG.md before fixing.
```

### Opus — implement a checklist item
```text
Follow prompts/FLIGHT_TRACKER_MASTER_PROMPT.md. Task: milestone <MX>, checklist item
"<item text>". Constraining docs: <list from the plan>. Implement, verify
(fmt/clippy/test), tick the item, update plans/CURRENT_STATUS.md, commit.
```

### Sonnet — tests/fixtures from spec
```text
Follow prompts/FLIGHT_TRACKER_MASTER_PROMPT.md. Task: write the tests specified in
docs/10_TEST_PLAN.md §<section> for <module>. The spec is the contract — no live network,
fixtures under tests/fixtures/. Do not modify non-test code except to fix compilation of
test hooks; if production code seems wrong, record it in plans/CURRENT_STATUS.md as a
blocker instead of fixing it.
```

### Sonnet — mechanical maintenance
```text
Follow prompts/FLIGHT_TRACKER_MASTER_PROMPT.md. Task: <mechanical task, e.g. "import
runways.csv per docs/08 schema">. Keep the diff scoped; no design changes; verify with
cargo test -p <crate>; update plans/CURRENT_STATUS.md.
```

## Session-splitting guidance

- Never mix a *decision* task and an *execution* task in one session — the decision changes
  what execution should read, and context balloons.
- Default to no subagent and normally cap a checklist item at one. Choose its model explicitly:
  Sonnet for mechanical/spec-driven work, Opus only for genuinely tricky implementation.
  Subagents never spawn subagents; cross-lane issues return to the orchestrator.
- If a Sonnet session hits a genuine design question, it stops and records the question
  (master prompt "blocked" rule) — it does not improvise architecture.
