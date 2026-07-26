# Testing Infrastructure

Status: current repository reference
Scope: the test commands, measured budgets, fixtures, corpora, and harnesses
that exist in this workspace today.

This document records current implementation facts: what each tool is and how
to run it. For rules that should guide future test design and placement, see
[Rust Testing Policy](testing_policy.md). For the *process* of working a
`umber2-johp` canonical/oracle divergence with these tools -- diagnosis order,
oracle hierarchy, fix discipline, gates, and the glossary defining that
vocabulary -- see [Canonical Divergence Working Contract](canonical_divergence_workflow.md).
This document does not restate that process.

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
Generated-input stabilization uses the hermetic shared fixtures under
`tests/corpus/stabilization`: native unit tests consume them directly, while
the wasm-bindgen browser suite runs the same bytes and compares binary output,
generated files, pass counts, and typed fixed-point failures with the native
surface.
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

The explicit stepwise recent-arXiv validation tier is:

```bash
cargo build -q --profile test -p umber --bin umber
UMBER_ARXIV_FORMAT=/path/to/pdflatex.umberfmt \
UMBER_ARXIV_DISTRIBUTION=/path/to/verified/texlive-snapshot \
  scripts/run-stepwise-arxiv-census.sh
```

The runner is serial and gives every paper one process through
`scripts/run-umber-guarded.py`, with cumulative engine fuel, wall-time,
aggregate-RSS, process-group TERM-to-KILL, reap, and survivor enforcement.
It defaults to `scripts/pdftex-arxiv-recent-sample-100.tsv` and the matching
gitignored source archives under `third_party/arxiv-recent-sample-100`.
Before running, it derives each entrypoint and source identity directly from
the pinned archive bytes. Identity records the archive hash, normalized
member-manifest hash, member count, and entrypoint instead of hashing a mutable
directory. Each child receives a new exact archive extraction in an ordinary
temporary directory. Mutation, missing members, and generated extras fail the
hermetic tooling tests.
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
second full compilation. `UMBER_ARXIV_LIMIT=1` selects the first row of the
recent sample.

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

Macro-invocation provenance has an assertion-bearing state performance tier:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml \
  --bench state_budgets provenance_memory/macro_long_run_arena_growth
```

Before timing, the benchmark expands 2,048 calls with 16-token bodies and
fails above 64 retained bytes per invocation. The charge includes archived
packed keys plus chunk and affine key-index metadata. Production admits at
most 1,048,576 record charges, a 64 MiB logical provenance-record budget;
excess diagnostic history degrades to unknown rather than aborting execution.

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

Its `--oracle tex82 --profile initex-eight-bit` and `--oracle etex26
--profile compatibility+extended-eight-bit` modes own pinned live reference
builds outside the correctness tier. Both reuse the hash-verified
TeX Live 2025 source cache offline, record the source/change/tool/platform and
executable identities under `target/`, and compare clean with instrumented
ordinary outputs. The e-TeX mode additionally verifies the distinct
compatibility and leading-`*` extended INITEX profiles, validates the complete
base schema-v1 event matrix in both, validates the focused expansion and
command-core extension matrix with compatibility exclusion, checks its
primitive-owner audit against canonical `etex.ch`, and repeats each base and
extension trace plus generated effect bytes deterministically.

The TeX82 workflow includes the committed `tex82/command-transitions-v1`
fixture gate; the explicit `--fixture` selector names the same gate. It
validates the contract-v1 manifest and hermetic bundle under
`tests/corpus/command`, audits every executable-matrix behavior against the
committed stream, focused sources, canonical citations, and ordinary-output
ownership, then requires the sources, stream, terminal, normalized log,
status, DVI, and generated effect to regenerate byte-for-byte. `tex-oracle`
unit tests consume that same committed bundle and the two pinned matrices
without a live TeX executable.
The focused source set includes separate legal and non-normal EOF programs so
the hermetic bundle distinguishes every TeX82 scanner-status recovery.

The `--oracle pdftex14027 --profile initex-etex-eight-bit` mode performs the
corresponding pinned pdfTeX 1.40.27 build. It gates DVI/PDF smoke artifacts,
the shared command
matrix, and a focused exact-eight-bit expansion/scanner matrix; proves the
549-primitive inventory as 391 shared TeX/e-TeX declarations plus 158 audited
pdfTeX additions with bidirectional primitive-to-matrix ownership; compares
clean and instrumented logs, status, DVI, PDF, and generated writes; parses
the smoke and state PDFs through the independent Hayro normalizer; and repeats
all three semantic traces plus state PDF projections byte-for-byte.

`--oracle all --profile canonical [--offline]` is the aggregate
cross-engine transparency gate. Before building, it validates the pinned
regeneration contract, exact source-manifest and fixture-audit hashes,
repository-owned inputs, event schema, canonical profiles, and committed
TeX82 fixture audit. It emits an uncommitted aggregate build record only after
all three workflows and the live TeX82 fixture comparison pass.
`--validate-only` performs the same hermetic identity, schema, and fixture
audit preflight without acquiring or building tools.

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
Synthetic PDF parser and importer inputs use the dependency-free
`test_support::pdf_fixture` classic-xref writer. It deterministically checks
indirect-object offsets and stream lengths while leaving complex object-stream
syntax to committed externally generated fixtures.
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

The independent host-tool gate is versioned as
`scripts/check-pdf-external.sh`. Its qpdf 12.3.2 matrix checks representative
classic trailers, all three object-compression policies, imported PDF and
raster/alpha/DCT images, Type 1/TrueType/PK/subset/tagged fonts, annotations,
forms, and navigation actions. Separately, Poppler 25.08.0 re-renders every
committed Umber PDF and compares it with the pinned PGM (exactly for ordinary
cases and with gray-value delta two for font cases); font extraction must also
match the committed UTF-8 bytes. Run `scripts/check-pdf-external.sh --local`
for development. A missing tool produces an explicit skip only in this mode;
an installed tool with the wrong version still fails. CI and release jobs must
install the pinned qpdf and Poppler versions and run
`scripts/check-pdf-external.sh --ci`, where missing tools and every validator
warning are fatal. `UMBER_PDF_VALIDATOR`, `UMBER_PDF_RENDERER`, and
`UMBER_PDF_EXTRACTOR` may select explicit executable paths.

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

The Story and Gentle callbacks also scan fixed-width provenance records after
execution, print invocation count, macro-attributed retained bytes,
bytes-per-invocation, and total provenance retention, and fail above the same
64-byte per-invocation budget as `state_budgets`. This scan is outside macro
expansion and therefore does not require profiling-only hot-path counters.

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

### Canonical Story Regression Gate

`e2e_conformance_story_canonical` in the same `crates/umber/tests/it/e2e_conformance.rs`
protects the `umber2-johp` epic's first canonical/reference byte-identical DVI
milestone (commit 5eed4dc3): the canonical `tex-command`/`CanonicalEngineSession`
architecture's assembled DVI for `story.tex` must remain byte-identical to real
pdfTeX's output, normalized only the same way `e2e_conformance_story` already
is. It is kept alongside, not instead of, the legacy `e2e_conformance_story`
test while the migration is in progress; both share the exact same staged
fixture directory (`parity_harness::run_named_fixture_document`, the same
`plain.tex`/`story.tex`/`hyphen.tex`/TFM staging the legacy in-process runner
consumes) and the same `plain_inputs_available` oracle-presence check, so it
skips with an explicit message when the locally generated `tests/corpus/e2e`
oracle or external corpus inputs are absent, exactly like every other e2e
case, and never reports a failure for a missing local oracle.

```bash
cargo test -p umber --test it e2e_conformance_story_canonical -- --nocapture
```

Unlike the legacy runner (`EngineSession` over `ExecutionContext`/
`InputResolver`/`FontResolver`), this test drives
`umber::CanonicalEngineSession` directly: it seeds the staged directory into a
memory `World` (via the shared `staged_world` helper both runners now use),
installs the canonical primitive tables with
`tex_expand::install_expandable_primitives`/
`tex_exec::install_unexpandable_primitives` (matching
`examples/first_failure_locator.rs`'s proven-working setup rather than the
CLI's `prepare_run_stores`), registers the staged job wrapper as an authored
root, and answers `\input`/font resource suspensions from a `StagedDirResourceHost`
that reads the same staged files the legacy resolvers do. It does not perform
the legacy test's macro-invocation-provenance budget check: that budget is a
legacy `EngineSession`/`ExecutionContext` observation that does not yet have a
canonical-session equivalent (see `umber2-johp.75`).

Under the `cargo test --tests`/`profile.test` (`opt-level = 1`) build this test
uses, Story's canonical run is fast (a few seconds), not the roughly 50-second
debug-build cost tracked separately in `umber2-johp.74`; the whole gate adds
on the order of 2 seconds to `cargo test -p umber --test it`'s wall time
alongside the other e2e cases.

On a mismatch it fails through the exact same `parity_harness::compare_dvi_files`
byte-identity contract as the legacy test, reporting the divergent page and
DVI opcode and writing a triage bundle under
`target/conformance-triage/story.tex/`. This was verified directly: temporarily
corrupting one assembled byte before the DVI comparison made
`e2e_conformance_story_canonical` fail with an exact byte/page/opcode mismatch
while `e2e_conformance_story` (legacy) kept passing; reverting the corruption
restored both to green. See the diagnosis order in [Canonical Divergence
Working Contract](canonical_divergence_workflow.md#2-diagnosis-order) for the
differential-tracer/first-failure-locator recipe to use once this gate
actually fails on a real regression.

## Canonical Command-Core Diagnostics

Two tools locate a `umber2-johp` command-core divergence or failure. This
section describes what each tool is, what it requires, and what it prints.
For the order to run them in, why the retired Umber implementation is never
an oracle, and what to do with the result, see the diagnosis order in
[Canonical Divergence Working Contract](canonical_divergence_workflow.md#2-diagnosis-order).

### Differential Tracer

```bash
cargo run -q -p tex-command-stream -- --repository .
cargo run -q -p tex-command-stream -- --repository . --max-divergences 50
cargo run -q -p tex-command-stream -- --repository . --realign-window 128
```

Run this from the repository root. It replays the committed
`tests/corpus/command/tex82` fixture registry through the instrumented
command boundary and compares the translated `tex-oracle` event stream
against the expected trace. It is fully hermetic (no external corpus,
distribution, or live TeX tool required) and never invokes a reference
engine.

It reports a ranked WORKLIST, not just the first divergence:

- Exit `0`: it prints nothing; no divergence against the fixture registry.
- Exit `1`: prints up to `--max-divergences` ordered divergences (default
  `DEFAULT_MAX_DIVERGENCES` = 20), one `[index] fixture <name> ... [kind]`
  entry per line pair, in stream order across every registered fixture. Two
  entry shapes share this ordered list:
  - A stream mismatch: the expected event, the actual observed event, and
    source context, labeled with a cheap structural `kind` (for example
    `command_identity_mismatch`, `command_operand_mismatch`,
    `event_kind_mismatch`, `mutation_mismatch`, `stream_truncated_early`) so
    same-kind entries can be batched without re-running the engine. The
    header line also carries the resynchronization the comparator applied and
    the cascade that resynchronization absorbed:

    ```text
    fixture <name> diverged at event 11375 (observed event 11385)
      [event_kind_mismatch] (resync: 1 oracle event(s) dropped by Umber;
       suppressed 32 cascade event(s))
    ```

    The observed index is printed only once it has drifted from the oracle
    index. See "Stream alignment" below for what each resync means.
  - A contained replay failure (`engine panicked` or `replay failed`): a
    command-core `ExecError` or a Rust panic during that fixture's replay is
    caught (`catch_panic`/`ReplayFailure`) and reported as its own ordered
    entry with the fixture, the event index it occurred after, and the
    failure's message (a panic's message and source location, exactly as
    the default panic hook would have printed -- `RUST_BACKTRACE=1` still
    produces a full backtrace on stderr). It does not abort the run: fixture
    replay continues afterward for any fixture ordered after it, up to the
    divergence budget. A panic outside a fixture's replay proper (fixture
    loading, suite/contract validation, argument parsing) is not contained
    and still aborts the process with the ordinary Rust panic exit code.

See `docs/command_semantic_fixtures.md` and `docs/alignment_brace_semantics.md`
for the fixture registry and event schema this replays and compares against,
and `tools/AGENTS.md` for what the tool does and does not do.

#### Stream alignment

The comparator (`tools/tex-command-stream/src/compare.rs`) treats the pinned
oracle stream and the observed stream as two sequences to be aligned, not as
two index-parallel arrays. Index-aligned comparison is only correct while
both streams agree on how many events each delivery produces; one dropped or
extra event otherwise turns every later index into a mismatch, and one root
defect fills the whole per-fixture budget with entries that say nothing new.

Every event splits into an alignment KEY and a PAYLOAD.

- The key is identity: the event kind, the canonical command identity
  (command name, control-sequence spelling, raw/expanded delivery) and its
  source position, and every structural transition -- input push/retire/stop,
  condition push/branch/pop, alignment transitions, token-list
  splice/complete, macro argument vs. activation.
- The payload is content: operands, scanner results, mutation keys and
  values, align state, token lists, diagnostic arguments.

A command's source position is part of its identity because long runs of
like-catcode characters are otherwise indistinguishable, and a shifted stream
could confirm a realignment against the wrong occurrence.

From that split the comparator produces one of these resyncs per entry.

- `payload differs, streams stay aligned` -- same key, different payload. A
  content-only defect; nothing was skipped and nothing cascades.
- `N oracle event(s) dropped by Umber` -- the oracle emitted N events Umber
  never produced.
- `N extra Umber event(s)` -- Umber emitted N events the oracle never
  produced.
- `N oracle event(s) replaced by M Umber event(s)` -- a short edit run; one
  replaced by one is an ordinary substitution.
- `structural: ... rejoined at <anchor> after skipping ...` -- nothing
  confirmed inside the window; the anchor fallback rejoined the two streams.
- `structural: ... no shared anchor; comparison of this fixture stopped here`
  -- neither the window nor an anchor confirmed a repair.
- `one stream ended with N event(s) remaining in the other` -- one stream ran
  out first.

On a key mismatch the comparator runs a wavefront search confined to a window
of events on each side, visiting candidates in ascending edit distance (and,
within one distance, ascending oracle skip) so the smallest repair wins
deterministically. A candidate is accepted only after a run of consecutive
key-equal pairs confirms it. When fewer than that many pairs remain before
the end of a stream, all remaining pairs must agree; skipping both streams to
their ends with no agreeing pair at all is a fork, not a repair.

If nothing confirms inside the window, the divergence is structural and one
anchor resync is attempted: both streams are scanned forward for the nearest
shared high-salience boundary -- an input-stack push/retire/stop, or the
first delivery attributed to a new source line -- and the same confirmation
is required there. If that also fails, comparison of that fixture stops and
says so. The bias is deliberate: cascade noise is visible, but a real defect
hidden behind an over-eager realignment is not.

This is not a global minimum-edit-distance diff, and deliberately so. Myers
is `O(ND)` and Gentle's trace is over 100 000 events; worse, a global minimum
would happily pair oracle event 700 with observed event 40 000 when that
minimizes total edits. Every search here is local, bounded, and paid at most
once per reported divergence, so the run stays linear in the streams.

`suppressed N cascade event(s)` counts the mismatches plain index-aligned
comparison would have reported over the stream region this entry covers --
from this entry's oracle index up to the next reported entry's, or to the end
of the streams for the last entry -- not counting the entry itself. It is the
cascade the entry stands in for, and it is how to tell one root site from
many: as of this writing the three document traces report 230, 269, and 501
entries where index-aligned comparison would report roughly 95 000, 96 000,
and 922 000.

#### Alignment tunables

Three flags bound the search. Widen them when a suspected repair is larger
than the defaults; narrow them to prove a reported realignment is not an
artifact of an over-generous bound.

- `--realign-window` (default 64) -- half-width of the wavefront search, in
  events per stream.
- `--realign-confirm` (default 8) -- consecutive key-equal pairs required to
  accept a realignment.
- `--anchor-scan` (default 4096) -- events scanned forward on each side by
  the structural anchor fallback.

A window of 64 costs `O(window^2)` key comparisons in the worst case, paid at
most once per reported divergence, and comfortably spans every repair shape
this epic has produced (a missing backup push, a duplicated raw/expanded
delivery pair, a macro activation expanded one level too far) while staying
far below the distance at which a confirmed match would be coincidence rather
than the same point in the document. A confirmation run of 8 is far more than
the two or three events that repeat by chance inside a run of like-catcode
characters, and small enough that a genuine repair immediately followed by a
second independent defect still confirms, leaving the second defect to be
reported on its own. The anchor scan is only reached once the window search
has already failed, so 4096 events -- roughly a page of document activity --
is generous on purpose; at most 64 anchors per side are considered.

All three flags take a positive integer and reject anything else with a usage
error.

#### Registries

The tracer replays two registries in this order.

1. **Committed fixtures** under `tests/corpus/command/tex82` -- today the
   single synthetic `tex82/command-transitions-v1`. Always present, fully
   hermetic, font-independent.
2. **Full-document traces** under `tests/corpus/command/tex82-documents` --
   `plain` (plain-format bootstrap alone), `story`, and `gentle`, each running
   `\input plain` plus the corpus document through real INITEX TeX82. These
   are _generated on demand and gitignored_: one plain trace is about 17 MB
   and Gentle's is about 156 MB, so committing them would add roughly 190 MB
   of exactly-reproducible generated bytes to the repository. A document
   whose trace tree is absent prints a one-line skip notice on stderr and is
   not a failure, so the tracer stays usable on a checkout that has never
   built the oracle.

Both registries produce the same ordered worklist entries; document
divergences follow committed-fixture divergences in the report.

Once the document tier is present, run the tracer through the `test` profile
(`opt-level = 1`) rather than the plain `dev` profile -- replaying hundreds of
thousands of document events unoptimized takes minutes where the `test`
profile takes seconds:

```bash
cargo run-dev -p tex-command-stream -- --repository . --max-divergences 50
```

#### Generating the full-document traces

```bash
scripts/build-tex82-oracle.sh --offline          # once, builds the pinned oracle
scripts/build-tex82-document-traces.sh           # all three documents
scripts/build-tex82-document-traces.sh --document plain   # one document
```

`scripts/build-tex82-document-traces.sh` stages
`tests/tex82-documents/<name>/root.tex`, `third_party/corpus/{plain,<name>}.tex`,
`third_party/hyphen/hyphen.tex`, and every `third_party/fonts/*.tfm` into a
fresh run directory; runs the clean and instrumented oracle executables (plus
a third repeat run) and requires their ordinary terminal/log/status/DVI
channels to agree and the instrumented trace to be bit-identical across
repeats; then derives a fully validated `tex-oracle` fixture through
`tex-oracle-bootstrap` and publishes it, with the staged TFMs beside it under
`fonts/`. It performs no network I/O. A partial (`--document`) run prints its
records instead of rewriting the contract, so the pinned file can never
describe a half-regenerated tree.

Identity is pinned in the committed
`tests/tex82-document-trace-manifest.txt`:

```text
document NAME ROOT-SOURCE FIXTURE-MANIFEST-SHA256 EVENTS FONT-SET-SHA256
```

The fixture manifest digest transitively pins every source, output, and event
byte (`CommittedFixture::load` verifies each file against it), the event count
pins the trace's scale, and the font-set digest -- a SHA-256 over one
`name sha256` line per staged TFM in bytewise name order -- pins the exact
metrics canonical replay registers. A present trace tree that disagrees with
any of these fails the run loudly rather than silently becoming the new
expectation. The same font-set digest is also the fixture manifest's
`distribution_sha256`, because the staged metric set _is_ this tier's
distribution.

Because `CanonicalMainControl::resolve_font_resource` returns
`ExecError::MissingCanonicalFont` immediately instead of suspending, the
tracer registers the whole staged TFM set through
`CommandHostCapabilities::register_font` before the first replay step rather
than through a lazy resource-host retry loop. Replay is bounded by
`(registered input bytes + expected event count) * 2 + 64` deliveries;
exceeding that bound is a contained worklist entry, not an aborted run,
because a document that expands far more commands than it has source bytes
must still terminate under a defect.

### First-Failure Locator

`crates/umber/examples/first_failure_locator.rs` is a standalone diagnostic
entry point for the `umber2-johp` command-core migration, separate from the
DVI-parity Cargo tests above and from the `umber2-johp.28` production
migration itself. Use it for the live end-to-end front, when the differential
tracer's fixture registry does not cover the failing input -- for example, it
depends on live document/font/hyphenation material outside
`tests/corpus/command`.

It stages `third_party/corpus/{plain,<source>}.tex`,
`third_party/hyphen/hyphen.tex`, and the plain-format CM/`manfnt` TFMs into an
in-memory `World` (reusing the same `parity_harness::CORPUS_TFMS`/`locate_tfm`
resolution as the harness above), then drives them directly through
`umber::CanonicalEngineSession` -- no `tex-lex::InputStack`, no legacy
`Executor` -- so it exercises exactly the same canonical command-core path the
migration is converging:

```bash
cargo run --profile test -p umber --example first_failure_locator -- gentle
cargo run --profile test -p umber --example first_failure_locator -- story
```

Use `--profile test` (matching `cargo run-dev`'s alias) rather than the plain
`dev` profile: Gentle and Story are large documents, and an unoptimized
`opt-level = 0` debug build of the still-unmigrated canonical path can take
several minutes where the `test` profile's `opt-level = 1` finishes in
seconds.

It reports the first failure it hits: the live execution mode, the
`ExecError`/`CanonicalSessionError` rendered with provenance-resolved TeX
source context (`ExecError::format_with_provenance`), or, for a Rust panic,
lets the default panic hook report the Rust-side `file:line` origin (rerun
with `RUST_BACKTRACE=1` for a full backtrace). As a first-failure locator (see
the Glossary in [Canonical Divergence Working
Contract](canonical_divergence_workflow.md#glossary)), it can only show that
execution stopped, never that completed output is wrong. It intentionally
does not run under `cargo test`: the command core is mid-migration and this
locator is expected to fail on `gentle` until each earlier divergence in the
`umber2-johp` chain is fixed. `story` currently completes cleanly and is a
regression gate (see "Canonical Story Regression Gate" above): a new `story`
failure is a regression to fix immediately, not the divergence under
investigation. See the current open successor issue under the `umber2-johp`
epic (`bd show umber2-johp` for its children) for the earliest tracked Gentle
divergence it reproduces -- that issue ID advances every time a divergence is
fixed, so it is not pinned here -- and `docs/tex_command_core.md` for the
canonical command-core architecture it exercises.

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
