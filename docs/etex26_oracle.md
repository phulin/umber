# Canonical e-TeX 2.6 Oracle

Status: pinned build boundary for command-core conformance.

## Authority and profiles

The canonical e-TeX oracle is built from Knuth's `tex.web` and the NTS
project's e-TeX 2.6 `etexdir/etex.ch` as distributed in the immutable TeX Live
2025 source archive. The archive is pinned by SHA-512. The two ordered WEB
sources, Web2C change stack, translator inputs, and repository-owned inputs
are pinned individually by SHA-256 in
`tests/etex26-oracle-manifest.txt`.

e-TeX has one executable program and two canonical INITEX profiles.
Compatibility mode starts from the ordinary input name and does not install
extension primitives. Extended mode consumes a leading `*`, installs the
e-TeX primitives, and records `eTeX_mode=1`. The build publishes each profile
under a distinct name so callers cannot leave that semantic choice implicit:

```text
target/etex26-oracle/bin/umber-etex26-compatibility-oracle-clean
target/etex26-oracle/bin/umber-etex26-extended-oracle-clean
target/etex26-oracle/bin/umber-etex26-compatibility-oracle-instrumentation-ready
target/etex26-oracle/bin/umber-etex26-extended-oracle-instrumentation-ready
```

The profile executables are byte-identical aliases of the applicable clean or
instrumentation-ready program because the profile is selected by canonical
INITEX input, not a compile-time flag. Their profile identity additionally
covers the invocation. These are external Web2C reference tools: none is
Umber, none may resolve to the Umber CLI, and Cargo correctness tests neither
acquire nor execute them.

## Reproducible build

Run the supported fixture-tooling entry point:

```bash
scripts/regen-fixtures.sh --area etex26-oracle
```

The first run may acquire the pinned archive into the gitignored
`third_party/texlive-source` cache. Verify cached reuse without network access:

```bash
scripts/build-etex26-oracle.sh --offline
```

`UMBER_REF_TEXLIVE_SOURCE` may select an equivalent cache containing the
pinned archive, extracted `src`, and configured `build` directories.
`CARGO_TARGET_DIR` relocates outputs.
`UMBER_ETEX26_INSTRUMENTATION_CHANGE` may select another final change file;
the build record captures its path and hash.

The build first merges `tex.web` with the upstream e-TeX change into the
canonical e-TeX WEB program. It then applies the pinned Web2C portability
changes in their declared order. The instrumentation-ready build appends
`tests/etex26-oracle/instrumentation-ready.ch` as the final change. That file
is intentionally transparent: the following instrumentation task owns adding
schema-v1 events at this already-pinned seam. Canonical and generated upstream
files are never edited in place.

Every run rewrites `target/etex26-oracle/build-record.txt` with engine,
character, and INITEX profile identities; archive and manifest hashes; ordered
WEB and change hashes; generated final-change hashes; translator, host
toolchain, linker, and platform identities; and executable and smoke-output
hashes. A font-independent program proves the compatibility profile leaves
e-TeX primitives undefined and the extended profile exposes e-TeX 2.6. For
each profile, clean and instrumentation-ready terminal, normalized log, exit
status, and DVI output must be byte-identical.

Executable hashes are platform-specific because the compiler and system
linker are inputs. Reproducibility means the complete pinned source, ordered
changes, flags, profile invocation, and recorded host toolchain determine the
result; it does not assert one cross-platform binary digest.
