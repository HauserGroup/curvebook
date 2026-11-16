# Audit work log

> **Scope:** This log records manual, human-launched slice-audit and independent finding-review runs under [`README.md`](README.md) and [`workflow.md`](workflow.md); automated multi-agent roadmap or defect-remediation orchestration (`docs/ROADMAP.md`) is tracked in commit history and `ROADMAP.md`/`STATUS.md`, not here.

Record what was audited, who did it, when it happened, the exact code commit, the agent used, and the slash command invoked. Use one entry per audit or independent review pass. A second command or a different code revision gets its own entry, even for the same slice.

Issues remain the place to claim work. This log records actual work and its evidence; a claim alone does not mean a slice was audited. A completed audit means the findings document was submitted in a PR, not that its findings were accepted or that the code is free of defects.

## How to record a run

The agent includes the entry metadata in its findings document, or in its review comment for an independent review. It links that evidence from the slice issue. The coordinator copies the entry here in a separate documentation update. Audit PRs still contain exactly one findings document; agents do not edit this shared log in their audit branches or wait for it to be updated.

Append new entries under Entries. Preserve earlier entries; add a dated correction if a record needs changing. Record partial and blocked runs too, with their actual coverage. If historical metadata is unavailable, write `unknown` rather than guessing. Only backfill a historical run when a report, issue, or other evidence supports the entry.

Use timestamps with a timezone, such as ISO 8601 UTC. Record the full SHA of the code examined, which may differ from the report commit or PR head. Identify the agent application, model, and version when known, as well as the worker or session identity. Record the exact slash command actually invoked, for example `/code-review`, `/verify`, or `/simplify`; use `none` for a plain prompt and `unknown` when it cannot be established. Naming a command here does not assert that it is installed or that it was run. Every command used under this audit workflow remains limited to inspection and reporting.

Follow [`SECURITY.md`](../../SECURITY.md) for private findings. Keep sensitive details and private evidence out of this public log.

## Entries

No audit runs have been entered in this log yet. Existing reports are indexed in [`README.md`](README.md).

## Entry template

Copy this block under Entries for each run. Remove the empty-log sentence when adding the first real entry.

```markdown
### <round> <slice> <worker> <started timestamp>

- Round and slice: <IDs>
- What was audited: <paths, functions, boundary flows, or finding IDs>
- Who: <auditor identity and accountable human handle>
- Worker or session: <unique run identity or session reference>
- Started: <timestamp with timezone>
- Finished: <timestamp with timezone, or unknown>
- Audited commit: <full code SHA>
- Agent: <application; model; version where known>
- Slash command: <exact invocation, none, or unknown>
- Prompt or skill: <prompt source and revision, skill/version, or plain prompt>
- Coverage: <complete within stated scope, partial, or blocked; exclusions>
- Result: <PR submitted, review posted, partial, or blocked; brief summary>
- Evidence: <findings document or review-comment link>
- Tracking: <slice issue and PR links, or not opened with reason>
- Checks and limitations: <commands and outcomes, omitted checks, blockers>
```
