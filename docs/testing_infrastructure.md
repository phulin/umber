# Testing Infrastructure

Status: current repository reference
Scope: the test commands, measured budgets, fixtures, corpora, and harnesses
that exist in this workspace today.

This document records current implementation facts. For rules that should
guide future test design and placement, see [Rust Testing Policy](testing_policy.md).

---

## Local Gates And Budgets

The fixture-only, hermetic correctness tier is:

```bash
cargo test --tests
scripts/check-and-test.sh
```

These commands use the root workspace's default native correctness members.
Run `scripts/check-wasm.sh` for the browser adapter and `scripts/check-tools.sh`
for opt-in regeneration, profiling, and triage tools.
The WASM target reserves a 4 MiB linear-memory stack because retained compile
sessions exceed wasm-ld's 1 MiB default during Firefox retry and incremental
HTML coverage; native targets keep their platform stack policy.

The warmed `cargo test --tests` target is under 10 seconds on the current
macOS development workspace; investigate a sustained run above 15 seconds or
any default test that invokes live TeX. `scripts/check.sh` checks dprint and
rustfmt formatting, then runs clippy without rerunning tests; it has a warmed
two-minute local budget.
`scripts/check-and-test.sh` runs the default native correctness suite followed
by that quality gate.

Commands that execute Umber, including Cargo tests whose selected test enters
the engine, must run through the shared process-group guard when investigating
a hang or memory-growth failure. A targeted invocation is:

```bash
python3 scripts/run-umber-guarded.py \
  --timeout-seconds 120 --max-rss-mib 6144 --term-grace-seconds 5 -- \
  cargo test -q -p umber --test it TEST_NAME -- --nocapture
```

Use a smaller RSS or time limit whenever the fixture permits it. The guard
sums resident memory across the command's process group, sends TERM to the
whole group at either limit, waits no more than five seconds, sends KILL to the
whole group, reaps the leader, and fails if any group member survives. Exit 124
means a time or RSS limit fired; exit 125 means cleanup itself failed. Run
`scripts/test-run-umber-guarded.sh` to exercise the forced-timeout and RSS-limit paths. On macOS
the guard reads process-group membership and live resident size through `libproc`; on Linux it
reads `/proc`. These native paths avoid a global `ps` subprocess and run inside the development
sandbox without elevated permissions.
Compiler-only commands such as `cargo check`, rustfmt, and clippy do not need
the guard. The guard complements rather than replaces an explicit finite
engine expansion-fuel setting on the exercised `ExecutionContext`. Native
resource sessions accept the bounded `UMBER_ENGINE_FUEL` override; invalid or
hard-maximum-exceeding values fail before execution.

The explicit stepwise arXiv validation tier is:

```bash
cargo build -q --profile test -p umber --bin umber
UMBER_ARXIV_FORMAT=/path/to/pdflatex.umberfmt \
UMBER_ARXIV_DISTRIBUTION=/path/to/verified/texlive-snapshot \
  scripts/run-stepwise-arxiv-census.sh
```

The runner is serial and gives every paper one process through
`scripts/run-umber-guarded.py`, with cumulative engine fuel, wall-time,
aggregate-RSS, process-group TERM-to-KILL, reap, and survivor enforcement.
Before running, it verifies each shared source view exactly against its pinned
archive. Source identity records the archive hash, normalized member-manifest
hash, member count, and entrypoint instead of hashing a mutable directory. Each
child receives a new exact archive extraction in a temporary directory, and
the canonical view is reverified after the child exits. Mutation, missing
members, and generated extras fail the hermetic tooling tests.
`RESOURCE_ENGINE_ACCEPTED` marks the transfer of accepted state to detached PDF
finalization in that process. A later map, encoding, PFB, PK, or PDF-lowering
failure therefore remains a finalizer outcome without recompiling the paper.
The TSV records both phase outcomes, replay telemetry, resource and engine time,
mutually exclusive accepted-run host phases, nested resolver/cache phases and
hit counts, estimated finalizer time, and guard status; failed rows retain
stable clusters.

The `profile.test` build is the optimized profile used by `cargo run-dev` and
shares its `target/debug/umber` artifact. A plain `cargo build` replaces that
path with the unoptimized development profile and is not a valid census binary.
The run identity records the exact binary path and hash. Row receipts retain
startup/format restore, engine, resource wait, VF lowering, font-usage, PDF
object/font embedding, image parse/copy, decode, transform, encode/cache,
serialization, materialization, and whole-run timings.

Each completed row has an atomically published JSON receipt under `rows/`.
Rerunning with the same binary, format, distribution manifest, sample, source
tree, limits, and mode rehashes its artifacts and skips it. Only an interrupted
row repeats. Changed identity or damaged evidence stops instead of mixing runs.
The explicit verified local distribution prevents fallback to the hosted pin.
The default is offline; set `UMBER_ARXIV_OFFLINE=0` for a warm cache-filling
run.

After a complete warm run, invoke the same results directory with
`UMBER_ARXIV_OFFLINE=1 UMBER_ARXIV_VERIFY_ONLY=1`. No child is launched: the
verifier rehashes immutable inputs and all durable row artifacts, then writes
`offline-verification.json`. This uses the native acquisition contract that an
acquired distribution object is digest verified and persisted in the
content-addressed cache before engine use, so attestation does not require a
second full compilation. `UMBER_ARXIV_LIMIT=1` selects `1609.01918` first.

Snapshot scaling has a separate explicit performance tier:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench snapshot_budgets
scripts/check-snapshot-budgets.sh
```

The Criterion command records the small/large latency rows. The script enforces
the low-noise latency ratio and requested-allocation retention budgets described
in [Snapshot Performance](snapshot_performance.md). Neither belongs in the
default cargo-test tier because its workload deliberately materializes large
input, page, mode, stream, hyphenation, provenance, and Unicode code-table
state.

Classic BibTeX has its own release-only performance and persistence tier:

```bash
scripts/check-classic-bibtex-budgets.sh
```

It checks fixed cold-compilation, cache-hit, native-session, and browser
WASM-session ceilings against the committed classic corpus. The precise
workloads, retained-cache caps, pinned compatibility identity, extensions, and
Phase 9/epic exit audit are recorded
in [Classic BibTeX Compatibility Inventory](classic_bibtex_inventory.md).

Incremental edit mapping and convergence have a separate deterministic fuzz
tier:

```bash
scripts/test-incremental-fuzz.sh
```

The wrapper runs the ignored `tex-incr` 1,000-edit scripted test, comparing
the incremental DVI with a fresh cold execution after every revision. It stays
outside the default Cargo tier because of its intentionally long edit
sequence.

Whole-engine Gentle profiling has a separate persistent in-process runner:

```bash
scripts/profile-gentle.sh
```

It preloads external corpus and font inputs into a structurally shared memory
World, performs a warm-up, then repeats fresh engine sessions without
per-iteration temporary-directory or host-file staging. The script builds an
optimized symbolized binary and saves the Samply profile under
`target/profiles/`. Its incremental matrix separately verifies slow,
interaction, fast suffix-adoption, and break-dependency hlist-rebreak paths.
See [Profiling Umber with Gentle](profiling.md) for its controls and measured
boundary.

## Fixture Regeneration

`scripts/regen-fixtures.sh` is the sole live-reference rewrite path. It builds
`tools/fixturegen` for text/native and PDF fixture updates and `tools/refexec`
for DVI fixture updates. Its `--area pdf` mode requires pdfTeX 1.40.27 and
Poppler `pdftoppm` 25.08.0; its `--area fonts` mode owns the explicit live
`tftopl` cross-check and does not rewrite fixtures.

See `tests/AGENTS.md` for the supported areas and cases, required tools,
copied support files, and validation performed after a rewrite.

The bibliography compatibility scaffold has one `bib-engine` Cargo
integration binary. It verifies all committed files below
`tests/corpus/bib/upstream-2.22` against a machine-readable SHA-256 manifest
that pins upstream commit `74252e608e5f8115375c532eb25416430a9f52eb` and the
Artistic-2.0 license. Its assertion-level xfail helpers cover exact strings,
bytes, deep values, and structured plus rendered diagnostics; a comparison
that unexpectedly matches is an XPASS and fails the test. Refreshing the
verbatim upstream input set is an explicit live-reference operation through
`scripts/regen-fixtures.sh --area bib`, never an ordinary Cargo-test action.
The same binary currently contains 1,275 assertion-isolated strict xfails for
51 foundation, input, graph, names, sorting, labels, uniqueness, output, and
tool-mode upstream files. Their Rust modules retain the complete pinned test
sources and exact assertion expressions for audit; subprocess-oriented output
tests record the equivalent in-process session status, byte-output, and
diagnostic expectations. The validation loop is expanded to 53 independent
tests so one XPASS cannot hide later validation assertions.

Classic BibTeX has a separate committed corpus under `tests/corpus/bibtex`.
Its manifest pins the TeX Live 2025 archive, `bibtex.web`, `bibtex.ch`, merged
Pascal, WEB2C-generated C/header, kpathsea and build configuration, exact
reference executable, inputs, status/history, BBL, BLG, and terminal bytes.
Its inventory assigns implementation and test owners to all 4 AUX commands,
10 BST commands, 3 BIB commands, 37 built-ins, 4 predefined symbols, and the
diagnostic, limit, branch, and upstream-test families. Ordinary tests audit
those committed bytes and owners only. The explicit
`scripts/regen-fixtures.sh --area bibtex` route builds and identity-checks the
pinned reference, executes it in an empty fixed-locale environment, refreshes
the outputs atomically, and reruns the hermetic audit.

The LaTeX format builder is a separate deterministic integration tier:

```bash
scripts/build-latex-format.sh --engine latex
scripts/build-latex-format.sh --engine pdflatex
```

Both modes verify that two clean format builds are byte-identical and that a
source-loaded smoke document exactly matches the corresponding format-loaded
document. The LaTeX mode compares DVI; the pdfLaTeX mode compares PDF. The
builder reads the common and mode-specific TeX Live input closure from
`tests/latex-source.lock`; its pdfLaTeX configuration is pinned locally in
`tests/latex/pdftexconfig.tex`. Generated formats and comparison artifacts
remain under `target/` rather than becoming repository fixtures.
All builder-started Umber and format-cache subprocesses reuse
`scripts/run-umber-guarded.py` with finite engine fuel, aggregate process-group
RSS and wall-time ceilings, and TERM-to-KILL/reap enforcement. Compiler-only
work remains outside that guard. Tune the bounded builder through the
`UMBER_LATEX_FORMAT_ENGINE_FUEL`, `UMBER_LATEX_FORMAT_MAX_RSS_MIB`, and
`UMBER_LATEX_FORMAT_TIMEOUT_SECONDS` variables rather than writing a separate
watchdog.
With `--publish-input-closure`, format metadata schema 2 also records the
canonical sorted request keys derived from that already verified trace. The
production snapshot builder uses this mode for both engines, stages local
configuration inputs into a pinned auxiliary root, and requires two complete
schema-3 publications to be byte-identical. Publisher tests cover closure
canonicalization, duplicate and size rejection, missing-key corruption, and
deterministic output without invoking live TeX tools.

## Committed DVI Corpora

The hand-authored distribution contract fixtures under
`tests/corpus/distribution` are consumed directly by both the dependency-free
`umber-distribution` Rust tests and authored JavaScript schema tests. They pin
strict manifest round trips and identical ordered acquisition jobs and typed
misses without network or TeX tooling.

The DVI corpora under `tests/corpus/dvi`, `tests/corpus/page`,
`tests/corpus/math`, `tests/corpus/align`, and `tests/corpus/leaders` commit TeX
source files plus `.expected.dvi` reference fixtures. The default `umber` cargo
tests run every `.tex` case in those areas against the committed DVI fixtures
without invoking live reference tools.

DVI regeneration runs the live reference engine through `tools/refexec`,
copies the pinned local CM TFMs and area support files, uses INITEX for the math
corpus, and rewrites raw reference DVI only when the existing
preamble-comment-only comparison detects a change.

## Committed PDF Corpus

`tests/corpus/pdf` commits minimal primitive-only sources, pinned reference
PDFs, deterministic Umber PDFs, normalized catalog/page/resource/content
structure, exact 72-dpi grayscale PGM renders, and renderer/hash attestations.
The `form_xobjects` case additionally canonicalizes decoded Form XObject
dictionaries and content operations, pins nested h/v/math placement and reuse,
and drives retained-session artifact/position/snap replay coverage.
Regenerate it only with `scripts/regen-fixtures.sh --area pdf` or
`scripts/regen-fixtures.sh --case pdf/<case>`.

Regeneration resolves object references and removes only byte-layout and
volatile metadata differences before comparing structure. It then renders
both PDFs with pinned Poppler and requires exact dimensions and pixels. The
ordinary cargo test invokes neither external tool: it rebuilds the exact Umber
bytes, normalizes the committed reference and current output, and verifies the
SHA-256 chain connecting both committed PDFs to the equal raster.

## External Document Corpus

External document inputs live outside committed fixtures. The line-oriented
`tests/corpus-manifest.txt` pins support files and documents by URL, fetched-byte
SHA-256, license determination, and redistributability flag. Runnable documents
also select a format source and pin the reference DVI SHA-256 after DVI preamble
banner normalization.

`scripts/setup-conformance-tests.sh` builds `tools/corpus-sync` to fetch or
verify those inputs under gitignored `third_party/corpus/`, then acquires the
remaining local support files and generates all four end-to-end DVI oracles.
Cached hash matches are a no-op. Fixture regeneration pins
`SOURCE_DATE_EPOCH=1783604160` and `FORCE_SOURCE_DATE=1` so date-sensitive
documents have stable DVI body bytes. Once setup completes, the conformance
tests consume only local files and require no network access.

Full external-document DVI parity is exposed as local-oracle-backed Cargo
integration tests:

```bash
cargo test -p umber --test it e2e_conformance_story -- --nocapture
cargo test -p umber --test it e2e_conformance_gentle -- --nocapture
```

Populate the external inputs and all Story, Gentle, TRIP, and e-TRIP DVI oracles with
`scripts/setup-conformance-tests.sh`. The generated `.expected.dvi` files are
gitignored licensing-sensitive derivatives and are not repository fixtures.

The shared `parity-harness` library stages inputs, calls the Cargo test's in-process Umber
runner, and byte-compares its normalized DVI with the local `tests/corpus/e2e`
oracle. Each document names a manifest-pinned
`format_source`; the harness stages that source, the document, hyphenation
input, and required TFMs, then feeds Umber a wrapper that inputs the format
source before the document through the ordinary input path.

This follows TeX82's ordinary `start_input` stack behavior (sections 23 and
29). Format dumping is a terminal INITEX cleanup operation (sections 46, 50,
and 51), not a way to continue into the document. The pinned modern
`plain.tex` contains no `\\dump`, so it can be loaded directly.

On fixture-hash drift, Umber failure, or mismatch, the harness writes a triage
bundle under `target/conformance-triage/<doc-name>/` with byte context,
page-limited dvitype-style disassemblies, a unified diff, tracing logs, and a
summary naming the divergent page and opcode when recoverable from DVI
backpointers. The `cargo test -p parity-harness self_test_bundle_pinpoints_page_and_opcode`
command exercises the bundle writer with synthetic DVI.
`scripts/regen-fixtures.sh --case e2e/story` and `--case
e2e/gentle` verify the manifest-pinned normalized reference hash before
rewriting either fixture.

## TRIP Corpus

The original Knuth TeX82 TRIP and e-TeX V2 e-TRIP workloads are end-to-end DVI
conformance tests that run conditionally when their local inputs and oracles
are present:

```bash
scripts/fetch-conformance-inputs.sh
scripts/fetch-conformance-inputs.sh --offline
cargo test -p umber --test it e2e_conformance_trip -- --nocapture
cargo test -p umber --test it e2e_conformance_etrip -- --nocapture
scripts/regen-fixtures.sh --case e2e/trip
scripts/regen-fixtures.sh --case e2e/etrip
```

`scripts/fetch-conformance-inputs.sh` acquires the shared hyphenation and font
inputs, reads `tests/trip-manifest.txt`, fetches exact official TRIP and e-TRIP
bytes into gitignored `third_party/trip/`, and verifies every SHA-256. The tests
use the pinned canonical `trip.tfm`, then run the documented INITEX and
format-loaded TRIP phases in process.

Cargo conformance tests do not launch Umber as a subprocess. Story and Gentle
call the engine directly through the staged fixture callback; TRIP and e-TRIP
share one in-process two-phase format helper.
`scripts/check-and-test.sh` checks these conditional corpus prerequisites before
starting the workspace gate and prints a warning naming every e2e case that
will be skipped and each missing file.

The Umber integration test gates only the final DVI. Generated logs, terminal
photo, and `tripos.tex` remain diagnostic outputs in the separate diagnostic
parity tier. Its oracle normalizes only the DVI preamble comment and otherwise
requires byte identity with the committed, locally pdfTeX-generated fixture.
Regeneration executes the two-phase workload from `trip.tex` and `trip.tfm`
and never copies the official `third_party/trip/trip.dvi`.

DVItype remains diagnostic. Failures write byte, page, opcode, and
disassembly context under `target/conformance-triage/trip/`. See
[TRIP](trip.md) for the exact source pins and normalization policy.

## Specialized Guards

`tex-out` owns the cross-crate page-output float guard. Its unit tests scan the
page node, packing, shipout lowering, artifact, DVI, and CLI DVI composition
sources and fail if float types or float rounding APIs enter that fixed-point
path. Its allowlist is limited to documented non-arithmetic fixture or
formatting false positives.

The explicit LaTeX tier is split by boundary. `scripts/check-latex-corpus.sh`
builds the pinned native format, runs the four base classes for three passes,
compares DVI and auxiliary artifacts with TeX Live 2026, and verifies the
30-input `tests/latex-runtime.lock` closure. This seed fixture is not the
production distribution: `scripts/build-texlive-snapshot.sh` enforces full
runtime inventory floors and package metadata hints. `scripts/check-latex-wasm.sh`
publishes that closure with the format, builds the real WASM package, and
exports that same format explicitly for the native run before requiring
byte-identical three-pass native/WASM article parity. Neither command belongs
in the ordinary workspace test tier because both intentionally build live
pinned distribution artifacts.

`scripts/test-publish-texlive-r2.sh` is the hermetic contract test for the R2
release command and runs in `scripts/check-and-test.sh`. Mock rclone and curl
boundaries cover dry-run behavior, failure followed by resumable rerun,
credential non-disclosure, bounded transfer/checker/retry flags, non-deleting
immutable copies, exact remote inventory checks, manifest-last ordering, and
public digest/CORS verification. It performs no network requests and uploads
nothing. The production staging and public origin are verified only by an
explicit coordinator invocation of `scripts/publish-texlive-r2.sh`.

The upstream LaTeX2e DVI tier is also explicit:

```bash
scripts/setup-latex-parity-tests.sh
scripts/setup-latex-parity-tests.sh --offline
scripts/check-latex-parity.sh --offline
scripts/check-latex-parity.sh --offline --format target/latex-parity/format/latex.fmt
scripts/check-latex-parity.sh --self-test-format-reuse
scripts/check-latex-parity.sh --self-test-reference-lookup
```

`tests/latex-parity-manifest.txt` pins the complete official
`release-2024-11-01-PL2` repository archive by commit, byte length, and SHA-256;
it does not pin individual support or test files. Setup extracts the unmodified
LPPL snapshot under gitignored `third_party/latex2e-parity/`, then derives every
same-stem standard-`.tlg` shipout candidate under `base`, `required/tools`,
`required/graphics`, and `required/amsmath`. The pinned tree yields 295
candidates. A live classic-LaTeX census emits DVIs for 286 of them and records
the nine exact manifest-pinned alternate-configuration paths separately;
unexpected reference DVI absence or presence fails the tier. The manifest retains
`base/testfiles/sx172785.lvt` in that 286-case reference-DVI cohort but skips it
explicitly as `unsupported-pdftex-primitives:pdfprotrudechars,rpcode`; this is
the only unsupported case, leaving 285 applicable classic-DVI comparisons.
Offline mode rejects a missing or changed archive cache without accessing the
network.

Without `--format`, the checker invokes the verified format builder exactly
once before entering the case loop. With `--format`, it invokes the builder
zero times. It hashes that pregenerated image, copies those exact bytes into a
fresh directory for every applicable reference/Umber pair, and each of the 285
Umber DVI runs loads the local copy with `--format latex.fmt`. The unsupported
case does not start an Umber run, so a complete current tier restores the
format exactly 285 times. The persistent
`target/latex-parity/last-run-format-receipt.txt` records the builder count,
source identity, and all 285 per-case identities; the fast self-test asserts
one build and three identical restores. A separate fast lookup self-test
accepts the declared snapshot, distribution, per-case, generated-state,
configuration, and format inputs while rejecting both a direct ambient input
and a symlink escape.

Each reference invocation starts from an empty environment with only the host
executable path and its deterministic clock/locale plus explicit kpathsea
settings. `TEXINPUTS` and `TEXFONTS` have no default-search suffix;
`TEXMFHOME`, `TEXMFCONFIG`, `TEXMFVAR`, caches, temporary files, and generated
fonts all point beneath that case's scratch root. The distribution's one
prebuilt `latex.fmt` remains an exact allowed file, not a general allowance for
ambient `texmf-var`. After every reference invocation, including non-DVI
configurations, the `.fls` recorder paths are canonicalized and must belong to
the case directory, pinned upstream snapshot, `texmf-dist`, isolated generated
state, or the two exact distribution configuration/format files. This check
runs before recorder-discovered input or TFM directories are passed to Umber.

The runner continues after individual
engine or DVI failures and writes complete persistent census lists to
`target/latex-parity/last-run-failures.txt` and
`target/latex-parity/last-run-non-dvi.txt`; explicit exclusions are recorded
separately in `target/latex-parity/last-run-skipped.txt`. The full-cohort
accounting requires tested plus skipped classic-DVI cases to equal the
manifest's 286-case reference-DVI count. Unless `--keep-work` is explicit, each
isolated reference/Umber pair is removed as soon as its result and compact
triage artifacts have been recorded, and the scratch root is removed on both
success and failure. This bounds temporary format-copy storage to one active
case instead of retaining all 285 copies after an expected census failure.
Reference and Umber cases have a 60-second timeout so one recovery loop cannot
stall the census without misclassifying the slower tools cases under
full-corpus load; set `UMBER_LATEX_CASE_TIMEOUT_SECONDS` to tune that explicit
tier locally.

Acceptance ignores transcript and process-status differences when an
intentional diagnostic still leaves a DVI. It removes stale DVI before every
pass and requires a newly emitted file, then normalizes only the existing DVI
preamble comment and otherwise requires byte identity. Mismatches write raw
DVI, first-byte context, page-limited disassemblies, and the divergent page and
opcode under `target/latex-parity/triage/<case>/`. This live TeX Live tier and
its roughly 74 MB format build remain outside ordinary Cargo tests.
