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

The first uses only the ordered upstream change stack. The second appends the
repository-owned final change file. That final file is currently an empty
instrumentation seam; command event instrumentation is owned by the follow-up
task. Canonical or generated upstream files are never edited in place.

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
`CARGO_TARGET_DIR` to relocate outputs. The later instrumentation task may set
`UMBER_TEX82_INSTRUMENTATION_CHANGE` to a final change file; the build record
captures its path and hash.

Each build rewrites `target/tex82-oracle/build-record.txt` with the archive,
manifest, ordered source/change files, final instrumentation change, generated
final changes, translator and host tool identities, platform identity, and
executable hashes. It then runs a font-independent INITEX smoke program through
both variants and requires byte-identical terminal and ordinary log output,
including the expected arithmetic result. The sole log normalization replaces
TeX's startup-banner host clock; no semantic message or diagnostic is changed.
Thus an instrumentation-ready build cannot silently alter ordinary TeX
behavior.

Executable hashes are platform-specific because the host compiler and system
linker are inputs. Reproducibility means identical pinned sources, ordered
changes, flags, and host toolchain produce the recorded executables; the build
record makes the resulting identity explicit rather than pretending one
cross-platform binary digest exists.
