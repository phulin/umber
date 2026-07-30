# Canonical TeX82 Oracle

Status: pinned build boundary for command-core conformance.

## Geometry schema v2 compatibility

Schema v1 is immutable and remains the format of all committed command fixtures. Schema v2 adds a detached `geometry` semantic event with three transitions: `hpack` and `vpack` contain finalized `width_sp`, `height_sp`, and `depth_sp`; `shipout` contains final `page_width_sp` and `page_height_sp`. Every value is a signed TeX scaled point (1/65536 pt). No node, pointer, memory, glue-ratio, selector, or output-driver identity is observable.

Schema version participates in both manifest and stream domain hashes. The
established v1 observer and decoder remain byte-for-byte compatible and do not
accept geometry records. The TeX82 geometry profile selects the v2 header
before emitting the detached `GeometryEvent` contract, while Umber observation
is separately opt-in through schema-v2 stream selection. Existing v1
observers, fixtures, JSON, headers, and identities remain byte-for-byte
unchanged.

The reference profile is a separately built writable executable and runs only
`tests/tex82-oracle/geometry.tex`. Its committed
`geometry-expected.jsonl` projection is a standalone canonical schema-v2 stream
that pins event order and signed scaled-point values. The microfixture covers
an explicit hbox and vbox, a font-independent paragraph line packed to
`\hsize`, an explicit shipment, and the end-of-job page-builder shipment. The
hooks observe the single finalized seams from tex.web §§633, 668, and 664:
`hpack`, `vpackage`, then `ship_out`. The native differential gate replays
this source with schema-v2 observation enabled and compares only the same
detached geometry projection. It reports dimension and page-total mutations as
`geometry_mismatch` with both signed scaled-point records.

No Gentle or other full document is part of this differential fixture. To run
the exhaustive full-document diagnostic manually after its local prerequisites
have been populated, generate the gitignored trace and use an unbounded-enough
comparison:

```bash
scripts/build-tex82-document-traces.sh --document gentle
cargo run -q -p tex-command-stream -- --repository . \
  --max-divergences 100000
```

That manual run retains the generated-document registry's schema-v1 command
stream and the committed schema-v2 geometry microfixture. Do not copy Gentle,
its generated trace, or any projection derived from it into the committed
native-test registry.

## Authority and identity

The canonical TeX82 oracle is built from Knuth's `tex.web` as distributed in
the immutable TeX Live 2025 source archive. The archive is pinned by SHA-512 in
`tests/tex82-oracle-manifest.txt`, which is the one identity the workflow
verifies; the selected WEB, the ordered Web2C change files, and the translator
inputs all come out of that archive. The Web2C changes are portability changes;
they do not make Umber part of the reference engine.

`scripts/build-tex82-oracle.sh` writes two explicitly named executables:

```text
target/tex82-oracle/bin/umber-tex82-oracle
target/tex82-oracle/bin/umber-tex82-oracle-instrumentable
target/tex82-oracle/bin/umber-tex82-oracle-geometry-profile
```

The first uses only the ordered upstream change stack. The second appends
`tests/tex82-oracle/instrumentation.ch`, the repository-owned final change
file. It emits schema-v1 JSON Lines to a dedicated `tex82-events.jsonl`
transport without using TeX's selector or transcript. Canonical or generated
upstream files are never edited in place.

The executables are external reference tools. Neither is Umber, neither may
resolve to the Umber CLI, and neither is invoked by Cargo correctness tests.

## Reproducible build

Run the supported fixture-tooling entry point:

```bash
scripts/regen-fixtures.sh --oracle tex82 --profile initex-eight-bit
```

The first committed semantic fixture adds the explicit selector:

```bash
scripts/regen-fixtures.sh --oracle tex82 --profile initex-eight-bit \
  --fixture tex82/command-transitions-v1
```

That selection validates the contract-v1 manifest, focused INITEX sources,
manifest-bound schema-v1 stream, canonical WEB citations, and ordinary
terminal/log/status/DVI/generated-effect observations under
`tests/corpus/command/tex82/command-transitions-v1`. It binds the live change
file's all-zero sentinel only in a temporary candidate and requires exact
agreement with the committed stream. The general repository contract is
documented in [`command_semantic_fixtures.md`](command_semantic_fixtures.md).
When adding a focused trace source, run the corresponding atomic bootstrap
instead:

```bash
scripts/regen-fixtures.sh --oracle tex82 --profile initex-eight-bit \
  --fixture tex82/command-transitions-v1 --bootstrap-fixture [--offline]
```

This is the only exception to validating the prior fixture before live
regeneration. It derives a separate candidate's focused sources, clean ordinary
outputs, bound semantic stream, manifest, and contract digest from the pinned
oracle, validates the candidate with both coverage matrices, and publishes
only after that validation. The ordinary selector continues to reject stale or
corrupt committed artifacts.

`umber2-alfh.2` split the single `command-transitions-v1` fixture -- which had
grown to 14 sources and 2.2 MB of events -- into one minifixture per
independent behavior, enforced by `crates/tex-oracle/src/minifixture_budget.rs`
(10 sources, 4 KiB of source bytes, and 8,000 events; a fixture that grows past
that ceiling fails `--bootstrap-fixture` rather than being committed). The
`tex82_fixtures` table in `scripts/build-tex82-oracle.sh` names every
selector's entry source and companions explicitly; the TeX82 builder stages
only that row's own sources into its run, not every `.tex` sibling in
`tests/tex82-oracle/`. `command-transitions-v1` itself kept `transitions.tex`
and the companions its input-stack, scanner-status, and physical-EOF-recovery
behavior inherently needs across nested files (`transitions-child.tex`,
`input-recovery.tex`, and the five `input-eof-*.tex` legal/EOF programs);
`case-shift.tex`, `expansion-macros.tex`, `alignment-delivery.tex`,
`off-save.tex`, and `scanner-conditionals.tex` (with its
`scanner-conditionals-eof.tex` companion) each became their own one-or-two-source
fixture under `tests/corpus/command/tex82/<name>-v1/`. Each split fixture has
its own `tests/tex82-oracle/<name>-v1-semantic-matrix.txt` and
`<name>-v1-audit-matrix.txt`, registered as `fixture`/`fixture-audit` rows in
`tests/oracle-regeneration-manifest.txt`; `--fixture tex82/<name>` and
`--bootstrap-fixture` work identically for every one of them, and
`crates/tex-oracle/src/suite.rs`'s `validate_tex82_command_trace_suite`
discovers the committed selector set from `tests/corpus/command/tex82` itself
rather than from a hardcoded list, so registering another split fixture needs
only its own directory and contract rows, not a code change. The legacy single-file
`tests/tex82-oracle/semantic-event-matrix.txt` and `fixture-audit-matrix.txt`
have been retired now that every family they described has an equivalent
per-fixture home. `crates/tex-command/tests/it/character_input.rs`'s
`CANONICAL_MATRIX` concatenates the six per-fixture
`<name>-v1-semantic-matrix.txt` files in their place, so it still reads every
family directly as a pinned external cross-check.
The focused source pins TeX's four job-clock parameters before shipout so the
ordinary DVI preamble remains exact even though the canonical `onlyTeX`
program does not consume Web2C's reproducible-clock environment variables.

The first run may acquire the pinned archive into the gitignored
`third_party/texlive-source` cache. After acquisition, pass `--offline` to
that same command to forbid network access. `--validate-only` checks the
cross-engine contract, source-manifest shape, repository-owned input hashes,
event schema, and engine/profile identity without building.

Set `UMBER_REF_TEXLIVE_SOURCE` only to select an equivalent cache containing
the pinned archive, extracted `src`, and configured `build` directories. Set
`CARGO_TARGET_DIR` to relocate outputs.
`UMBER_TEX82_INSTRUMENTATION_CHANGE` may select another final change file; the
build record captures its path and hash.

Each build rewrites `target/tex82-oracle/build-record.txt` with the archive,
manifest, ordered source/change files, final instrumentation change, generated
final changes, translator and host tool identities, platform identity, and
executable hashes. It then runs a font-independent INITEX smoke program and the
focused command-event matrix through both variants. Terminal, normalized log,
exit status, and DVI remain byte-identical. The instrumented transition
program runs twice and must emit byte-identical traces and ordinary outputs; the
`tex-oracle` validator checks canonical JSON encoding, schema,
manifest-field shape, and contiguous sequence numbers. Focused checks require
raw and expanded delivery, source and token input lifecycle, backup, scanner
status, outer-validity recovery insertion, terminal stop, and termination
ordering. Dedicated source inputs additionally gate physical-line
normalization and M/N/S tokenization, per-character catcode effects, canonical
`^^` reduction, `get_token` consumers, nested parameter replay, normal EOF,
and defining/matching/absorbing/aligning/skipping EOF. The EOF diagnostics
record scanner status and exact right-brace, `\par`, frozen `\cr`, or frozen
`\fi` recovery. Macro-focused checks additionally require completed arguments and
activations, delimiter matches and overlap recovery, parameter conversion,
replacement completion, ordinary and expanded `scan_toks` completion, and
direct `\the` token-list splices. The focused `expansion-macros.tex` child
also gates one-delivery `\noexpand`, `\expandafter` ordering, defined and
undefined `\csname`, TeX82 conversion primitives, leading, undelimited,
delimited, `#{`, and nine-parameter matching, long and non-long paragraph
behavior, nested parameter replay, and `\def`/`\gdef`/`\edef`/`\xdef` plus
prefix meanings. Its ordinary log records independent `\meaning` and `\show`
observations. Scanner/conditional checks additionally
require typed integer, scaled-dimension, glue, internal-value, and token-list
results; condition push, limit-change, branch, and pop transitions; skipped
delimiter and `\ifcase` progress; and evaluating-limit plus incomplete-skip
recovery events. The dedicated `scanner-conditionals.tex` input pins
signed-radix, fractional physical-unit, infinite glue-order, and internal
token-list values and independently exposes their TeX spellings through
`\message`. It also covers `\if`, `\ifcat`, raw `\ifx`, selected `\ifcase`,
recursive evaluation of a condition operand, skipped nested conditions and
braces, evaluating-limit recovery, extra-delimiter recovery, and a focused
EOF child. Alignment checks require preamble
start/finish and repetition, one-based nested ownership and exact
suspend/resume, literal-brace `align_state` changes, control-sequence group
aliases, exact backup correction, tab/`\span`/`\cr`/`\crcr` interception,
u/v/omit-template push and retirement, `\noalign`, and recovery. The dedicated
`alignment-delivery.tex` input covers ordinary and nested alignments,
font-independent rule output, repeated preambles, literal braces, group
aliases, `\omit`, `\span`, `\noalign`, missing and extra `#`, missing left and
right braces, and an extra tab. Its messages independently expose u/v-template
execution and `\noalign`; its shipped rule boxes make alignment packaging
visible in DVI bytes. Alignment nesting is a semantic counter maintained by
the detached observer; no alignment record, input level, or `mem` address
enters the stream. The focused `off-save.tex` child enters a simple group,
issues `\endgroup`, and then emits a progress marker. This makes the canonical
trace observe `off_save` above bottom level as a backup plus inserted right
brace and replay, then observe the replay's bottom-level drop without an
input-frame loop. pdfTeX's corresponding transition input uses the same
construction and schema-v1 event fragments. The sole ordinary-log normalization
replaces TeX's startup-banner host clock; no semantic message or diagnostic is
changed. Assignment-command-scoped observation at the committed `eq_define`,
`geq_define`, `eq_word_define`, and `geq_word_define` seams emits typed
meaning, catcode, code-table, parameter, and register mutations without
including internal eqtb maintenance. Committed message, expanded write,
output-stream open/close, and successful DVI shipout seams emit ordinary
effects. The transparency comparison also covers the exact generated write
file.

## TeX82 command-event matrix

The six per-fixture
`tests/tex82-oracle/{command-transitions,case-shift,expansion-macros,alignment-delivery,off-save,scanner-conditionals}-v1-semantic-matrix.txt`
files are the executable audit inventory, one per split fixture. Every row
names a required schema-v1 boundary, its canonical focused input, the stable
final-change seam that observes the committed transition, and a fixed
canonical-JSON fragment. The build fails on a malformed row or an absent
observation, so adding a schema boundary without mapping it cannot silently
weaken coverage.

| Family                      | Focused program                            | Stable canonical seams                                                                                                      |
| --------------------------- | ------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------- |
| command and input           | `transitions.tex`, `transitions-child.tex` | `get_next`, `get_x_token`, `x_token`, input push/retirement, terminal stop                                                  |
| recovery and scanner status | both transition inputs                     | `back_input`, `check_outer_validity`, and scoped status entry/restoration                                                   |
| macros                      | `transitions.tex`                          | committed delimiter match/recovery, stripped argument, and activation seams in `macro_call`                                 |
| token lists                 | `transitions.tex`                          | parameter conversion, direct `\the` splice, and completed `scan_toks` collection                                            |
| scanners                    | `transitions.tex`                          | successful integer, dimension, glue, and internal-value returns                                                             |
| conditions                  | both transition inputs                     | frame link/unlink, exact-frame limit updates, branch selection, and EOF recovery                                            |
| alignments                  | `alignment-delivery.tex`                   | alignment ownership, preamble lifecycle, literal-brace accounting, delimiter interception, template lifecycle, and recovery |
| off-save recovery           | `off-save.tex`                             | `off_save` replay with inserted closer, then bottom-level drop and bounded progress                                         |
| mutations                   | `transitions.tex`                          | assignment-scoped committed meaning, catcode, code-table, parameter, and register writes                                    |
| ordinary effects            | `transitions.tex`                          | committed message, expanded write, output-stream open/close, and successful DVI shipout                                     |
| final ordering              | `transitions.tex`                          | terminal input stop followed by termination before observer close                                                           |

The matrix covers every TeX82-applicable schema-v1 `StateTarget` and every
non-termination `EffectKind`, in addition to ordered termination.
Each split fixture's matching `<name>-v1-audit-matrix.txt` supplies the other
half of the audit. It assigns each semantic family to the exact manifest
citation and useful ordinary-output channels. Hermetic validation proves that
every event row occurs in the committed stream, every focused source is used,
the event and audit family sets agree, and every manifest citation, source,
and output is owned. Every matrix pair's hashes are pinned by the
`fixture-audit` rows in `tests/oracle-regeneration-manifest.txt`; live builds
and Cargo correctness tests therefore cannot accept a mutually drifting
source/event/manifest bundle.

The live change file writes the all-zero manifest identity as an explicit
unbound sentinel. It is not a committed fixture identity. Cross-engine
fixture integration owns binding traces to complete canonical manifests; no
trace with the sentinel may be committed as an oracle.

## Full TRIP observer profile

The focused command profile deliberately serializes complete canonical command
and token values. The official full TRIP workload instead uses the separately
built `umber-tex82-oracle-trip-profile` executable. Its schema-v1 stable
observations are only ordered DVI `shipout` effects (page numbers), followed by
the terminal-input `stop` and engine `terminate` effects. A run that reaches
TeX82 §93 `succumb` instead retains its fatal diagnostic immediately before
the same engine termination, preserving the terminal reason and
`fatal_error_stop` history without fabricating an input stop. This bounded
profile does not serialize command, control-sequence, token-list, macro,
scanner, mutation, diagnostic, or textual-effect payloads. Validate it with
`tex-oracle-validate --tex82-trip-profile`; that validator checks canonical
JSONL and only the declared stable observations and final ordering.

Run `scripts/test-tex82-trip-observer.sh` after fetching TRIP inputs to build
the pinned oracle offline, execute both phases twice, validate deterministic
profile streams, and compare statuses, terminal transcripts, logs, and final
DVI against the clean oracle. The DVI comparison normalizes exactly the
preamble-comment payload; its length byte, every other preamble byte, and the
complete body and postamble must match. The bounded profile's coverage is
therefore the stable ordered `shipout` effects plus terminal `stop` and engine
`terminate`, rather than the unstable full command stream.

The aggregate transparency gate is:

```bash
scripts/regen-fixtures.sh --oracle all --profile canonical --offline
```

It writes `target/oracle-regeneration/build-record.txt` only after all three
engines pass their clean/instrumented artifact comparisons and schema-v1 trace
gates, and after the TeX82 live source, event stream, and ordinary artifacts
match the fully audited committed fixture.

Executable hashes are platform-specific because the host compiler and system
linker are inputs. Reproducibility means identical pinned sources, ordered
changes, flags, and host toolchain produce the recorded executables; the build
record makes the resulting identity explicit rather than pretending one
cross-platform binary digest exists.
