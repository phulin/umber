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
- `src/profile.rs` and `src/profile/tests.rs`: public semantic character values,
  immutable engine/character profiles, capabilities, stable fingerprints, and
  focused value/identity tests.
- `src/state.rs`: persistent command state and discardable runtime ownership.
- `src/command.rs`: public opaque, ephemeral current-command representation.
- `src/error.rs`: private command error and resource-need representation.
- `src/input/source.rs`, `src/input/source/tests.rs`: public host-neutral
  source-registration inputs and errors plus private immutable backing,
  source cursors, and focused registration tests.
- `src/input/lines.rs`, `src/input/lines/tests.rs`: exact physical-line
  splitting, TeX line normalization, byte/scalar cursor and range accounting,
  and focused line-contract tests.
- `src/input/tokenizer.rs`, `src/input/tokenizer/tests.rs`: canonical
  token-at-a-time exact-byte and separately identified UnicodeExtended M/N/S
  tokenization, semantic control-sequence spelling, profile-specific
  superscript notation, invalid-character recovery steps, byte/scalar ranges,
  and focused conformance tests.
- `src/input/levels.rs`, `src/input/levels/tests.rs`: dense source/token-list
  levels, stored/transient/argument payload ownership, orthogonal delivery and
  retirement behavior, replay explanations, and focused ownership tests.
- `src/input/`: remaining private backup and summary state machines.
- `src/processor/`: public borrow-only processor facade with private raw
  delivery, expansion, scanner-status, and alignment orchestration.
- `src/scanners/`: private typed scanner family.
- `src/primitives/`: private static TeX82, e-TeX, and pdfTeX dispatch families.
- `src/macro_call.rs`: private canonical scalar macro matcher.
- `src/conditionals.rs`: private independent condition-stack machine.
- `src/scan_toks.rs`: private canonical token-list scanner.
- `src/provenance.rs`: private command provenance construction.
- `src/observation.rs`: private aggregate read observation.
- `src/snapshot.rs` and `src/snapshot/tests.rs`: command snapshot, quiescent
  summary ownership, and focused internal roundtrip/rejection tests.
- `tests/`: external dependency, visibility, and capability-boundary tests.
  Character/input integration coverage binds the exact shared-domain tokenizer
  to the pinned TeX82 fixture and compile-fail gates profile immutability.
