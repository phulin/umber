# Canonical pdfTeX 1.40.27 Oracle

Status: pinned build boundary for command-core conformance.

## Authority and identity

The canonical pdfTeX oracle is built from `pdftexdir/pdftex.web` as
distributed in the immutable TeX Live 2025 source archive. The archive is
pinned by SHA-512. The WEB source, ordered Web2C and configured SyncTeX change
stack, translator inputs, build description, runtime configuration, and
repository-owned inputs are pinned individually by SHA-256 in
`tests/pdftex14027-oracle-manifest.txt`.

The canonical invocation is INITEX with e-TeX extensions explicitly enabled.
Its command character profile is exact eight-bit input. The repository tooling
publishes two names:

```text
target/pdftex14027-oracle/bin/umber-pdftex14027-oracle-clean
target/pdftex14027-oracle/bin/umber-pdftex14027-oracle-instrumented
```

The clean executable uses only the declared upstream change stack. The
instrumented executable appends
`tests/pdftex14027-oracle/instrumentation.ch` and then
`tests/pdftex14027-oracle/extension-instrumentation.ch`. The first change
ports the complete shared TeX82/e-TeX schema-v1 command trace; the second
observes pdfTeX expansion/scanner extensions at stable committed seams.
Neither edits canonical or generated upstream engine files. The detached
trace is written to `pdftex14027-events.jsonl`.

Both executables are external Web2C reference tools. Neither is Umber, neither
may resolve to the Umber CLI, and Cargo correctness tests neither acquire nor
execute them.

## Reproducible build

Run the supported fixture-tooling entry point:

```bash
scripts/regen-fixtures.sh --area pdftex14027-oracle
```

The first run may acquire the pinned archive into the gitignored
`third_party/texlive-source` cache. Verify cached reuse without network access:

```bash
scripts/build-pdftex14027-oracle.sh --offline
```

`UMBER_REF_TEXLIVE_SOURCE` may select an equivalent cache containing the
pinned archive and extracted `src` tree. The workflow uses a dedicated
`build-pdftex14027` directory inside that cache so its configure state and
archive-owned libpng, zlib, xpdf, and kpathsea artifacts do not alias the
TeX82/e-TeX oracle builds. `CARGO_TARGET_DIR` relocates outputs.
`UMBER_PDFTEX14027_INSTRUMENTATION_CHANGE` may select another final change;
the build record captures its path and hash.

Every run verifies the archive and every manifest entry before translation.
It applies the exact configured TeX Live change order, including SyncTeX
changes selected by the canonical pdfTeX build profile, and builds static
archive-owned library inputs. No network access occurs after the archive is
present, and acquisition remains outside correctness tests.

The observer makes the merged pdfTeX WEB exceed TANGLE's historical
16-bit token/name-byte and expansion-stack capacities. The workflow therefore
derives a private capacity-only TANGLE C source from the pinned generated
translator, widens only those storage types and limits, and records both its
source and executable identities. This private translator changes no WEB
tokenization or pdfTeX source semantics.

`target/pdftex14027-oracle/build-record.txt` records:

- engine, e-TeX extension, character, and invocation profiles;
- archive, manifest, WEB source, ordered change, generated final-change, and
  repository final-change hashes;
- configure flags and the fixed source epoch;
- translator, compiler, linker-driver, shell, make, platform, archive-owned
  static-library, linked system-library, and executable identities; and
- normalized smoke log plus exact DVI and PDF hashes for both variants; and
- transition terminal, normalized log, status, DVI, generated-write, and
  schema-v1 trace hashes.

The smoke programs validate the canonical 1.40.27 banner and numeric
`\pdftexversion`, e-TeX 2.6 extended mode, expansion/arithmetic, and shipout.
The DVI program emits a font-independent one-page DVI. The PDF program fixes
the creation-information policy, trailer ID, and compression controls before
a font-independent one-page PDF. Terminal bytes, startup-clock-normalized log
bytes, exit status, and DVI/PDF bytes must match between clean and instrumented
executables.

`tests/pdftex14027-oracle/semantic-event-matrix.txt` is the executable coverage
inventory for shared delivery, input lifecycle, backup and recovery, scanner
status, macros, token-list construction, scanners, conditions, alignments,
typed mutations, effects, and termination. The focused transition program
runs through clean and instrumented executables; their ordinary channels and
generated write bytes must match. The trace is schema-validated, matrix-gated,
and reproduced byte-for-byte by a second instrumented run. Events use semantic
names and values only and exclude allocation, `mem`, pool, path, selector,
stream-slot, input-index, and helper-call identities.

`extension-primitive-audit.txt` completes the canonical primitive inventory.
The build derives 391 shared TeX/e-TeX primitives from the pinned `tex.web`
and `etex.ch`, requires the audit's 158 rows to equal the remaining
`pdftex.web` declarations exactly, and classifies those rows as command-core
or executor/backend. Command-core expansion rows must name a boundary in
`extension-event-matrix.txt`; state, enquiry, and effect rows identify the
dependent phase-2 matrix. The focused extension program covers exact
eight-bit conversions, comparisons, file queries and hashing, regex captures,
primitive enquiry, extended predicates, deterministic randomness, and profile
identity. Clean/instrumented ordinary channels and generated bytes match, and
the extension trace is schema-validated and reproduced byte-for-byte.

Executable hashes are platform-specific because the compiler, linker, system
libraries, and platform are inputs. Reproducibility means the complete pinned
source, ordered changes, flags, profile invocation, library artifacts, and
recorded host toolchain determine the result; it does not assert one
cross-platform binary digest.
