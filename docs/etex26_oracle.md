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
target/etex26-oracle/bin/umber-etex26-compatibility-oracle-instrumented
target/etex26-oracle/bin/umber-etex26-extended-oracle-instrumented
```

The profile executables are byte-identical aliases of the applicable clean or
instrumented program because the profile is selected by canonical
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
changes in their declared order. The instrumented build appends
`tests/etex26-oracle/instrumentation.ch` as the final change. That detached
observer ports the complete TeX82-applicable schema-v1 command contract to
e-TeX's canonical seams, including changed assignment, pseudo-file,
protected-alignment-lookahead, and conditional-unwind paths. Canonical and
generated upstream files are never edited in place.

Every run rewrites `target/etex26-oracle/build-record.txt` with engine,
character, and INITEX profile identities; archive and manifest hashes; ordered
WEB and change hashes; generated final-change hashes; translator, host
toolchain, linker, and platform identities; and executable and smoke-output
hashes. A font-independent program proves the compatibility profile leaves
e-TeX primitives undefined and the extended profile exposes e-TeX 2.6.
The focused transition program and child input run in both profiles. Each
instrumented trace is schema-validated, checked against
`semantic-event-matrix.txt`, and reproduced byte-for-byte by a second run.
The matrix covers delivery, input lifecycle, recovery, scanner status, macro
matching and activation, token-list collection, scanners, conditions,
alignments, assignment mutations, ordinary effects, and termination. For each
profile, clean and instrumented terminal, normalized log, exit status, DVI,
and generated write bytes must be byte-identical.

A second focused program, `tests/etex26-oracle/extensions.tex`, takes an
explicit compatibility-exclusion branch and exercises the extended profile's
version and revision identity, `\readline`,
protected macro construction, protected expanded-token-list suppression,
`\unexpanded`, `\detokenize`, expanded `scan_toks` construction,
`\scantokens`, `\everyeof`, all four expression scanners, glue component and
conversion enquiries, extended conditionals and `\unless`, current
group/condition and interaction enquiries, all six sparse-register families,
last-node, font-character, and paragraph-shape enquiries, the tracing
parameters, and remaining command-core integer state, including
`\predisplaydirection` and `\TeXXeTstate`. Sparse
register events name the semantic family and register number; they never expose
tree nodes, sparse-array indexes, or allocation identity. Its extended trace is
gated by `extension-event-matrix.txt`; extension-only fragments must be absent
from the compatibility trace. `extension-primitive-audit.txt` is checked
against the complete primitive inventory parsed from the pinned canonical
`etex.ch`; every command-core-owned primitive must name a matrix boundary,
while executor-owned node, list, paragraph, math, and diagnostic primitives
name their existing focused parity gate. The extended body is stored unexpanded
behind the profile branch so undefined extension conditionals cannot disturb
compatibility-mode skip nesting. Both profile traces are schema-validated and
reproduced byte-for-byte, and clean versus instrumented terminal, normalized
log, status, DVI, and generated extension-effect bytes must agree. Canonical
e-TeX 2.6 does not define the later `\expanded` primitive: this oracle records
its expanded token-list construction seam, while the pdfTeX oracle owns the
primitive itself.

The all-zero manifest identity in the live stream is an explicit unbound
sentinel, not a fixture identity. The final change records semantic names and
values only; allocation addresses, input-stack indexes, pool indexes, physical
paths, and helper-call identity remain outside the event stream.

Executable hashes are platform-specific because the compiler and system
linker are inputs. Reproducibility means the complete pinned source, ordered
changes, flags, profile invocation, and recorded host toolchain determine the
result; it does not assert one cross-platform binary digest.
