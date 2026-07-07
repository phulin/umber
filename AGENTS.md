# Repository Guidance

This project is a reimplementation of TeX in Rust, with a goal of eventually providing feature parity and a faster, more modern, more portable version of the original.

The repository uses progressive disclosure: read this file first, then the nearest nested `AGENTS.md` before editing within a subdirectory; keep every `AGENTS.md` up to date whenever source files or subdirectories are added, removed, or repurposed.

The project also uses bd (beads) for issue tracking; see below for full instructions.

## General Instructions
- Keep committing as you go; commit in logical chunks, and write good commit messages. You have to escalate privileges to commit.
- For long-running implementation goals, do not treat the work as complete until there is working parity on all test corpuses relevant to the goal and the overall implementation plan is complete.
- Make sure you are writing clean code; don't hesitate to do refactor commits if you find that a certain area of the code has gotten complex or difficult to understand.
- Don't worry about keeping changes "low-risk" or implementing only "narrow slices", as making clean code will sometimes require big, ambitious, cross-cutting changes, and reimplementing something from scratch means we will need to write complex new features.
- If you discover that a major subsystem is missing, prefer implementing it in one coherent pass instead of scattering partial fragments across many small changes; errors can be revised later.
- In general, try to keep source files short (goal is under roughly 600 lines, but it's okay if a file gets somewhat larger; test files can be as long as needed, they should only be split logically).
- Document todos and stubs in the code clearly with a TODO.
- For complex features, build design/technical documentation in advance and place in docs/ for your own planning and for reference later, but don't commit temporary task plans or notes.
- When writing code, prefer principled solutions, clean architecture, and fast, optimized implementation. Avoid hacks.
- Make sure you can run the test suite very quickly so we don't gate our progress on test su ite speed. Run `cargo test` with `--tests` so you don't run the doctests.

## Directory Map

- `.agents/`: project-local agent skills and coordination workflow guidance.
- `tools/`: Rust workspace tools for reference execution and corpus synchronization.
- `tests/`: committed fixtures and parity test definitions.
- `docs/`: architecture, phase, and design documents.
- `scripts/`: local development scripts and versioned git hook templates.
- `third_party/`: ignored reference downloads and external source archives.

## Beads Issue Tracker

Use Beads (`bd`) for durable task tracking in repositories that include it. Use the `beads` skill at `.agents/skills/beads/SKILL.md` (project install) or `~/.agents/skills/beads/SKILL.md` (global install) for Beads workflow guidance, then use the `bd` CLI for issue operations.

### Quick Reference

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work
bd close <id>           # Complete work
bd prime                # Refresh Beads context
```

### Rules

- Use `bd` for all task tracking; do not create markdown TODO lists.
- Run `bd prime` when Beads context is missing or stale. Codex 0.129.0+ can load Beads context automatically through native hooks; use `/hooks` to inspect or toggle them.
- Keep persistent project memory in Beads via `bd remember`; do not create ad hoc memory files.

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.
