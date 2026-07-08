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
- Prefer `#[cfg(test)] mod tests;` with separate `src/.../tests.rs` files for nontrivial crate-internal tests; use crate-level `tests/` for public-boundary, CLI, parity, fixture, replay, capability, and compile-fail tests. See `docs/testing_policy.md`.
- Document todos and stubs in the code clearly with a TODO.
- For complex features, build design/technical documentation in advance and place in docs/ for your own planning and for reference later, but don't commit temporary task plans or notes.
- When writing code, prefer principled solutions, clean architecture, and fast, optimized implementation. Avoid hacks.
- Make sure you can run the test suite very quickly so we don't gate our progress on test su ite speed. Run `cargo test` with `--tests` so you don't run the doctests.

## Directory Map

- `.agents/`: project-local agent skills and coordination workflow guidance.
- `crates/`: Rust workspace crates.
- `crates/tex-arith`: shared TeX scaled-point and TFM arithmetic.
- `crates/tex-state`: engine state layer substrate.
- `crates/tex-fonts`: immutable font metric parsing and TFM data.
- `crates/tex-exec`: stomach execution, mode nest, main-control dispatch, and h-mode material construction.
- `crates/umber`: CLI driver.
- `crates/test-support`: shared fixture and parity-test helpers.
- `tools/`: Rust workspace tools, including `refexec` for reference TeX execution.
- `tests/`: committed fixtures and parity test definitions.
- `docs/`: architecture, phase, and design documents.
- `scripts/`: local development scripts and versioned git hook templates.
- `third_party/`: ignored reference downloads and external source archives.

## Development

- Run `scripts/install-hooks.sh` once after clone to enable the versioned pre-commit hook.
- Use `scripts/check.sh` as the local gate before committing.
- For `UPDATE_FIXTURES=1` fixture regeneration, follow `tests/AGENTS.md`.

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

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:6cd5cc61 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->

<!-- BEGIN BEADS CODEX SETUP: generated by bd setup codex -->
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
<!-- END BEADS CODEX SETUP -->
