# Canonical pdfTeX 1.40.27 Oracle

Status: pinned build boundary for command-core conformance.

## Authority and identity

The canonical pdfTeX oracle is built from `pdftexdir/pdftex.web` as
distributed in the immutable TeX Live 2025 source archive. The archive is
pinned by SHA-512 in `tests/pdftex14027-oracle-manifest.txt`, which is the one
identity the workflow verifies. Everything the build reads out of that archive
-- the WEB source, the ordered Web2C and configured SyncTeX change stack, the
translator inputs, the build description -- is already covered by the archive
digest, and the repository-owned inputs are covered by the commit they live in.

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
`tests/pdftex14027-oracle/extension-instrumentation.ch` and
`tests/pdftex14027-oracle/state-instrumentation.ch`. The first change ports
the complete shared TeX82/e-TeX schema-v1 command trace; the second observes
pdfTeX expansion/scanner extensions and names pdfTeX state semantically; the
third observes the remaining state, enquiry, and PDF-facing effect seams.
Neither edits canonical or generated upstream engine files. The detached
trace is written to `pdftex14027-events.jsonl`.

Both executables are external Web2C reference tools. Neither is Umber, neither
may resolve to the Umber CLI, and Cargo correctness tests neither acquire nor
execute them.

## Reproducible build

Run the supported fixture-tooling entry point:

```bash
scripts/regen-fixtures.sh --oracle pdftex14027 \
  --profile initex-etex-eight-bit
```

The first run may acquire the pinned archive into the gitignored
`third_party/texlive-source` cache. After acquisition, pass `--offline` to
that same command to forbid network access. `--validate-only` checks the
cross-engine contract, source-manifest shape, repository-owned hashes, event
schema, and exact INITEX/e-TeX/eight-bit profile without building.

`UMBER_REF_TEXLIVE_SOURCE` may select an equivalent cache containing the
pinned archive and extracted `src` tree. The workflow uses a dedicated
`build-pdftex14027` directory inside that cache so its configure state and
archive-owned libpng, zlib, xpdf, and kpathsea artifacts do not alias the
TeX82/e-TeX oracle builds. `CARGO_TARGET_DIR` relocates outputs.
`UMBER_PDFTEX14027_INSTRUMENTATION_CHANGE` may select another final change;
the build record captures its path and hash.

Every run verifies the archive before translation. It applies the exact configured TeX Live change order, including SyncTeX
changes selected by the canonical pdfTeX build profile, and builds static
archive-owned library inputs. No network access occurs after the archive is
present, and acquisition remains outside correctness tests.

Editing the manifest itself is still a reviewed change: refresh the pdfTeX
row's manifest digest in `tests/oracle-regeneration-manifest.txt` and run the
validate-only preflight before the full workflow. The validators intentionally
reject a stale identity; this is a build-input refresh, not a semantic-trace
regeneration.

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
- transition, extension, and state terminal, normalized log, status, artifact,
  generated-write, and schema-v1 trace hashes.

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
`extension-event-matrix.txt`, whose rows identify the owning primitive
explicitly; matrix rows must map back to the same command-core audit entry.
State, enquiry, and effect rows are gated by `state-event-matrix.txt`. The
focused extension program covers exact
eight-bit conversions, comparisons, file queries and hashing, regex captures,
primitive enquiry, extended predicates, deterministic randomness, and profile
identity. Clean/instrumented ordinary channels and generated bytes match, and
the extension trace is schema-validated and reproduced byte-for-byte.

The focused state program covers every inventory-owned pdfTeX parameter,
token-list and dimension parameter, font code table, object-independent
enquiry, random/timer transition, committed PDF shipout, and generated write.
Font identities are semantic TFM names and sizes; inserted pdfTeX object
numbers are suppressed from recovery and command delivery. The host-dependent
elapsed-time value is represented by the stable `host_timer_sample` boundary
rather than by its numeric sample. Clean/instrumented terminal, normalized
log, status, PDF, and generated-write bytes match, and the state trace is
schema-validated and reproduced byte-for-byte.

PDF transparency has a second, artifact-independent gate. The host-only
`test-support` `pdf-normalize` command parses both the deterministic PDF smoke
artifact and the focused state artifact through the bounded Hayro probe,
projects their semantic catalog/page/resource/content structure without
preserving object allocation or byte layout, and requires clean and
instrumented projections to agree; the repeated state run must reproduce the
same projection. The build record captures the normalizer executable and
projection hashes separately from raw PDF hashes. PDF object structure remains
absent from command events.

The aggregate transparency gate is:

```bash
scripts/regen-fixtures.sh --oracle all --profile canonical --offline
```

It binds the regeneration contract and each engine build record in
`target/oracle-regeneration/build-record.txt` only after all clean/instrumented
transparency and schema-v1 trace gates pass.

Executable hashes are platform-specific because the compiler, linker, system
libraries, and platform are inputs. Reproducibility means the complete pinned
source, ordered changes, flags, profile invocation, library artifacts, and
recorded host toolchain determine the result; it does not assert one
cross-platform binary digest.

## Running a command-semantic minifixture through the oracle

`scripts/run-minifixture-oracle.sh` runs one or more
`tests/corpus/command-semantic/<domain>/<fixture>/manifest.json` cases through the
already-built instrumented executable above and captures every channel a
case's run can produce: terminal text, the raw and host-clock-normalized log,
the DVI/PDF page artifact, `status.txt` (exit code), any writer-effect file
the source itself opens (via `\openout`/`\write`, discovered rather than
assumed at a fixed name), and the schema-v1 `pdftex14027-events.jsonl` trace.
It never builds the oracle and performs no network access.

```bash
scripts/run-minifixture-oracle.sh --case main-control/eqtb-regions
scripts/run-minifixture-oracle.sh --all
```

The loaded raw-TeX82 batch uses the blessed regeneration entry point with an
explicit profile and reviewed exact-case allowlist:

```bash
scripts/regen-fixtures.sh --area command-semantic \
  --profile raw-tex82-loaded \
  --allowlist tests/command-semantic-oracle-profiles/raw-tex82-loaded.cases
```

That route removes any prior cached raw TeX82 image, constructs one
`production.fmt` with a separate `-ini`/`\dump` priming job, and reuses it for
every selected `-fmt=production` job. The logical oracle profile remains
`raw-tex82-loaded`; `production` is the format name shared with Umber's raw
TeX82 recipe and therefore the banner identity under comparison. A missing
dump or load is a hard failure; there is no INITEX fallback. The priming
transcript and log are construction evidence only and are never copied into a
case channel.

The allowlist contains exactly 90 loaded jobs: the original 35 scanner,
input-expansion, and conditional cases, plus 55 main-control cases. The
main-control cohort is every `main-control/*` fixture except
`hyphenation-data`, `hyphenation-errors`, and
`final-cleanup-end-or-dump`. Its job-local contract is one declared input,
three TFM resources, six terminal-interaction cases, and 51 empty plus four
file DVI channels. Capturing the cohort under genuine loaded framing changes
55 terminal banners, 55 normalized-log banners, and five event counts from
the obsolete INITEX captures; projections, status, DVI, effects, resources,
and interaction declarations do not change.

Each case is staged under `target/minifixture-oracle/<domain>/<case-id>/`
with its `source`, any `inputs` files (written as the manifest's exact
string, byte for byte), and any `font_inputs` TFM (copied from the
repository path the manifest names).

**Interaction mode.** The script passes the interaction mode declared by each
case, defaulting to `-interaction=scrollmode`, rather than the `batchmode` the
rest of this oracle's own smoke/extension/state fixtures use. Two constraints
select that default: `\read`-from-terminal and `\pausing` (used by the 11 cases
with a `terminal_lines` manifest field) are hard errors, respectively no-ops, under
`batchmode`/`nonstopmode` (tex.web requires `interaction>nonstop_mode`), so
those modes would misrun exactly the cases meant to exercise terminal
interaction. The tex82 default, `errorstopmode`, satisfies that but also
stops and prompts at _every_ error, not only the ones a case's
`terminal_lines` anticipates: verified directly, `main-control/font-definition`
(no `terminal_lines`) hits an ordinary error under `errorstopmode` and, with
no further terminal input queued, halts at "! Emergency stop." before
reaching the rest of the source. `scrollmode` is `>nonstop_mode` (so
`\read`/`\pausing` work) and still "omits error stops" like batch/nonstopmode
do, so an undeclared error just prints and the run completes.
`main-control/show-completion` declares `errorstopmode` plus a justification
in its manifest because it specifically exercises the `errorstopmode`-only
`?␣` prompt after `\showthe`; its terminal answer switches the engine to
scrollmode from that prompt. The runner reads this per-case declaration, so
the case is reproduced under the interaction mode its channels require.

**Profile fidelity.** The manifest `profile` field maps to an invocation as:

- `initex` (default) → `-ini`, no e-TeX.
- `etex-initex` → `-ini -etex`, the same invocation the rest of this oracle
  already uses.
- `etex-loaded` and `production` → **not reproduced**. Both need a genuinely
  loaded (`\dump`-ed and reloaded) format; `tools/tex-command-stream`'s
  in-process `SessionProfile::EtexLoaded`/`Production` build this by dumping
  and restoring an in-memory `Universe`, which has no analog here yet (a real
  `\dump` format file plus a `-fmt` invocation). The 3 `etex-loaded` and 2
  `production` cases are skipped with an explicit message rather than run
  under the wrong profile.

This entry point is intentionally separate from
`scripts/regen-fixtures.sh`'s `command-semantic` area, which derives its
per-channel contracts without a reference engine; wiring reference-engine
capture into that area is future work, not part of this script.
