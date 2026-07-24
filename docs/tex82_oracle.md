# Canonical TeX82 Oracle

Status: pinned build boundary for command-core conformance.

## Authority and identity

The canonical TeX82 oracle is built from Knuth's `tex.web` as distributed in
the immutable TeX Live 2025 source archive. The archive is pinned by SHA-512,
and the selected WEB, ordered Web2C change files, translator inputs, and
repository-owned inputs are pinned individually by SHA-256 in
`tests/tex82-oracle-manifest.txt`. The Web2C changes are portability changes;
they do not make Umber part of the reference engine.

`scripts/build-tex82-oracle.sh` writes two explicitly named executables:

```text
target/tex82-oracle/bin/umber-tex82-oracle
target/tex82-oracle/bin/umber-tex82-oracle-instrumentable
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
start/finish, one-based nested ownership and suspend/resume, `align_state`
changes, exact backup correction, tab/`\span`/`\cr` interception,
u/v/omit-template push and retirement, and preamble recovery. The focused
input covers ordinary and nested alignments, braces, `\omit`, `\span`,
`\noalign`, missing `#`, and extra `#`. Alignment nesting is a semantic
counter maintained by the detached observer; no alignment record, input level,
or `mem` address enters the stream. The sole ordinary-log normalization
replaces TeX's startup-banner host clock; no semantic message or diagnostic is
changed. Assignment-command-scoped observation at the committed `eq_define`,
`geq_define`, `eq_word_define`, and `geq_word_define` seams emits typed
meaning, catcode, code-table, parameter, and register mutations without
including internal eqtb maintenance. Committed message, expanded write,
output-stream open/close, and successful DVI shipout seams emit ordinary
effects. The transparency comparison also covers the exact generated write
file.

## TeX82 command-event matrix

`tests/tex82-oracle/semantic-event-matrix.txt` is the executable audit
inventory. Every row names a required schema-v1 boundary, its canonical
focused input, the stable final-change seam that observes the committed
transition, and a fixed canonical-JSON fragment. The build fails on a malformed
row or an absent observation, so adding a schema boundary without mapping it
cannot silently weaken coverage.

| Family                      | Focused program                            | Stable canonical seams                                                                                                      |
| --------------------------- | ------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------- |
| command and input           | `transitions.tex`, `transitions-child.tex` | `get_next`, `get_x_token`, `x_token`, input push/retirement, terminal stop                                                  |
| recovery and scanner status | both transition inputs                     | `back_input`, `check_outer_validity`, and scoped status entry/restoration                                                   |
| macros                      | `transitions.tex`                          | committed delimiter match/recovery, stripped argument, and activation seams in `macro_call`                                 |
| token lists                 | `transitions.tex`                          | parameter conversion, direct `\the` splice, and completed `scan_toks` collection                                            |
| scanners                    | `transitions.tex`                          | successful integer, dimension, glue, and internal-value returns                                                             |
| conditions                  | both transition inputs                     | frame link/unlink, exact-frame limit updates, branch selection, and EOF recovery                                            |
| alignments                  | `transitions.tex`                          | alignment ownership, preamble lifecycle, literal-brace accounting, delimiter interception, template lifecycle, and recovery |
| mutations                   | `transitions.tex`                          | assignment-scoped committed meaning, catcode, code-table, parameter, and register writes                                    |
| ordinary effects            | `transitions.tex`                          | committed message, expanded write, output-stream open/close, and successful DVI shipout                                     |
| final ordering              | `transitions.tex`                          | terminal input stop followed by termination before observer close                                                           |

The matrix covers every TeX82-applicable schema-v1 `StateTarget` and every
non-termination `EffectKind`, in addition to ordered termination.

The live change file writes the all-zero manifest identity as an explicit
unbound sentinel. It is not a committed fixture identity. Cross-engine
fixture integration owns binding traces to complete canonical manifests; no
trace with the sentinel may be committed as an oracle.

The aggregate transparency gate is:

```bash
scripts/regen-fixtures.sh --oracle all --profile canonical --offline
```

It writes `target/oracle-regeneration/build-record.txt` only after all three
engines pass their clean/instrumented artifact comparisons and schema-v1 trace
gates.

Executable hashes are platform-specific because the host compiler and system
linker are inputs. Reproducibility means identical pinned sources, ordered
changes, flags, and host toolchain produce the recorded executables; the build
record makes the resulting identity explicit rather than pretending one
cross-platform binary digest exists.
