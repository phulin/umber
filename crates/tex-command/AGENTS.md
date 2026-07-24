# tex-command Guidance

Read the repository-level `AGENTS.md` and `docs/tex_command_core.md` before
editing this crate.

## Crate Role

`tex-command` is the sole target owner of canonical source tokenization, input
levels, raw command delivery, expansion, macro calls, conditions, scanners,
alignment delivery, and static profile dispatch. It depends on `tex-state` and
must never depend on `tex-exec`; `tex-exec` consumes its completed
unexpandable commands.

Host capabilities are borrow-scoped through `CommandHostContext` and must
never enter snapshots, formats, durable summaries, or owned command state.
Private state-machine modules must not be widened for compatibility with
`tex-lex` or `tex-expand`.

## File Map

- `Cargo.toml`: dependency-light crate manifest and boundary-test support.
- `src/lib.rs`: intentionally small public facade and private module tree.
- `src/host.rs`: borrow-scoped, nonserializable host-capability boundary.
- `src/profile.rs`: private engine/character profile ownership.
- `src/state.rs`: persistent command state and discardable runtime ownership.
- `src/command.rs`: public opaque, ephemeral current-command representation.
- `src/error.rs`: private command error and resource-need representation.
- `src/input/`: private source, line, tokenizer, input-level, backup, and
  summary state machines.
- `src/processor/`: public borrow-only processor facade with private raw
  delivery, expansion, scanner-status, and alignment orchestration.
- `src/scanners/`: private typed scanner family.
- `src/primitives/`: private static TeX82, e-TeX, and pdfTeX dispatch families.
- `src/macro_call.rs`: private canonical scalar macro matcher.
- `src/conditionals.rs`: private independent condition-stack machine.
- `src/scan_toks.rs`: private canonical token-list scanner.
- `src/provenance.rs`: private command provenance construction.
- `src/observation.rs`: private aggregate read observation.
- `src/snapshot.rs`: private command snapshot and summary ownership.
- `tests/`: external dependency, visibility, and capability-boundary tests.
