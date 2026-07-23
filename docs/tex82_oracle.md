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
scripts/regen-fixtures.sh --area tex82-oracle
```

The first run may acquire the pinned archive into the gitignored
`third_party/texlive-source` cache. Verify reuse without network access with:

```bash
scripts/build-tex82-oracle.sh --offline
```

Set `UMBER_REF_TEXLIVE_SOURCE` only to select an equivalent cache containing
the pinned archive, extracted `src`, and configured `build` directories. Set
`CARGO_TARGET_DIR` to relocate outputs.
`UMBER_TEX82_INSTRUMENTATION_CHANGE` may select another final change file; the
build record captures its path and hash.

Each build rewrites `target/tex82-oracle/build-record.txt` with the archive,
manifest, ordered source/change files, final instrumentation change, generated
final changes, translator and host tool identities, platform identity, and
executable hashes. It then runs a font-independent INITEX smoke program and a
focused command/input/recovery program through both variants. Terminal,
normalized log, and exit status remain byte-identical. The instrumented
transition program runs twice and must emit byte-identical traces; the
`tex-oracle` validator checks canonical JSON encoding, schema,
manifest-field shape, and contiguous sequence numbers. Focused checks require
raw and expanded delivery, source and token input lifecycle, backup, scanner
status, outer-validity recovery insertion, terminal stop, and termination
ordering. Macro-focused checks additionally require completed arguments and
activations, delimiter matches and overlap recovery, parameter conversion,
replacement completion, ordinary and expanded `scan_toks` completion, and
direct `\the` token-list splices. Scanner/conditional checks additionally
require typed integer, scaled-dimension, glue, internal-value, and token-list
results; condition push, limit-change, branch, and pop transitions; skipped
delimiter and `\ifcase` progress; and evaluating-limit plus incomplete-skip
recovery events. The focused input covers nested conditions, `\ifx`, skipped
balanced braces, and EOF while skipping. The sole ordinary-log normalization
replaces TeX's startup-banner host clock; no semantic message or diagnostic is
changed.

The live change file writes the all-zero manifest identity as an explicit
unbound sentinel. It is not a committed fixture identity. Cross-engine
fixture integration owns binding traces to complete canonical manifests; no
trace with the sentinel may be committed as an oracle.

Executable hashes are platform-specific because the host compiler and system
linker are inputs. Reproducibility means identical pinned sources, ordered
changes, flags, and host toolchain produce the recorded executables; the build
record makes the resulting identity explicit rather than pretending one
cross-platform binary digest exists.
