# Classic BibTeX compatibility inventory

Status: Phase 10 systematic differential generator implemented. Its reference
gate is currently blocked by tracked BLG accounting parity work; real-world
corpus and remaining hardening work are tracked in Beads.

The reviewed architecture and phase exit criteria remain those in
`docs/classic_bibtex_bst.md` at commit `c676cfb0`. This inventory does not
replace or revise that design. Classic BibTeX and the existing
Biber-compatible implementation remain separate semantic backends behind one
resource, result, and project-orchestration facade. Classic `.bst` execution
must not be routed through `bib-graph`, `bib-sort`, `bib-label`, or
`bib-output`.

The machine-audited source of this census is
`tests/corpus/bibtex/inventory.json`. It names every AUX command, BST command,
BibTeX database command, built-in, predefined VM-visible symbol, diagnostic
family, reference capacity, relevant branch family, and upstream test, with
both an implementation owner and a test owner. The ordinary `bib-engine`
integration test fixes the exact names and counts so omissions cannot silently
enter later implementation phases.

## Pinned executable specification

`tests/corpus/bibtex/manifest.json` fixes the complete reference identity:

| Component             | Pinned identity                                                           |
| --------------------- | ------------------------------------------------------------------------- |
| Source distribution   | TeX Live 2025 `texlive-20250308-source.tar.xz`, SHA-512 in the manifest   |
| WEB source            | `texk/web2c/bibtex.web`, SHA-256 `38b9ba09…31ed`                          |
| Change file           | `texk/web2c/bibtex.ch`, SHA-256 `9bffb931…716`                            |
| Merged program        | `tangle bibtex bibtex` output `bibtex.p`, SHA-256 `a0362ee3…f79`          |
| WEB2C output          | exact `bibtex.c` and `bibtex.h` SHA-256 identities                        |
| Build                 | repository TRIP Web2C configuration, `-O2`, generated `c-auto.h` identity |
| File lookup           | pinned kpathsea `texmf.cnf`, isolated `BIBINPUTS=.` and `BSTINPUTS=.`     |
| Executable provenance | Darwin arm64, Apple clang 17.0.0, exact binary SHA-256                    |
| Runtime               | `LC_ALL=C`, `LANGUAGE=C`, otherwise empty environment                     |

The exact binary hash records the executable that produced the committed
bytes; the merged Pascal and generated C identities are the portable semantic
build identity. A different platform is not allowed to masquerade as that
recorded executable. A future reviewed platform identity must be added to the
manifest rather than weakening the current pin.

The change file is part of the specification. It changes dynamic capacities,
8-bit character acceptance, file opening and kpathsea lookup, line handling,
the banner and capacity report, output safety, command-line parsing, exit
status, and memory allocation. Auditing `bibtex.web` alone is insufficient.

## Construct census and ownership

The exact name lists and owner strings live in `inventory.json`; this table is
the review summary.

| Reference-owned surface       | Count | Primary implementation issue                   | Primary test boundary                          |
| ----------------------------- | ----: | ---------------------------------------------- | ---------------------------------------------- |
| AUX commands                  |     4 | `umber2-ild0.3`                                | classic AUX/VFS integration tests              |
| BST top-level commands        |    10 | `umber2-ild0.5`                                | `bib-bst` parser/compiler matrices             |
| BIB record commands           |     3 | `umber2-ild0.4`, `umber2-ild0.7`               | raw parser and classic database tests          |
| Built-in functions            |    37 | `umber2-ild0.8`, `umber2-ild0.9`               | VM matrix plus one focused matrix per built-in |
| Predefined VM-visible symbols |     4 | `umber2-ild0.7`, `umber2-ild0.8`               | database and VM tests                          |
| Diagnostic families           |    15 | phase-specific owners through `umber2-ild0.11` | typed and rendered diagnostic parity           |
| Reference/safety limits       |    18 | phase-specific owners through `umber2-ild0.13` | boundary and adversarial tests                 |
| Branch families               |    15 | phase-specific owners through `umber2-ild0.13` | differential branch/state coverage             |
| Upstream test programs        |    17 | `umber2-ild0.3` through `umber2-ild0.13`       | imported and focused parity cohorts            |

The 37 built-ins are the complete `n_equals` through `n_write` dispatch in the
merged program. The separate predefined field/variables `crossref`,
`sort.key$`, `entry.max$`, and `global.max$` are inventoried because the WEB
source calls them built-ins conceptually even though `fn_type` classifies them
as storage. The `default.type` fallback is owned by the VM lifecycle and
unknown-entry-type branch rather than counted as a 38th primitive.

Every built-in test owner must cover ordinary values, empty and missing
values, every wrong stack type, stack underflow, brace/control-sequence cases,
output and diagnostic ordering, compatibility character boundaries, and work
and output limits. Whole-style tests remain necessary for lifecycle, entry
context, mutation, stable sorting, and composed output.

## Diagnostics, limits, and branch coverage

Diagnostic ownership follows the stage that has enough context to produce the
event. AUX, BST, and BIB scanners retain their distinct recovery rules. The VM
owns stack underflow, wrong-type, entry-context, and invariant events. The
classic output/invocation boundary owns line wrapping, file-open behavior,
history rendering, and process status. Typed diagnostics are rendered into BLG
and terminal bytes only after semantic ordering is fixed.

The four reference histories are `spotless`, `warning`, `error`, and `fatal`.
Warnings and recoverable errors can still yield a complete BBL. In particular,
operand-stack underflow is a recoverable VM error: execution continues through
later entries and commands, preserves ordered stack/error log events, and the
classic command returns the reference error exit status. Infrastructure failure
remains outside this history model, and fatal partial files are never published
by a project transaction. `umber2-ild0.2` owns the typed history;
`umber2-ild0.11` owns exact terminal, BLG, status, and partial-artifact parity.
`top$` consumes and emits its literal as a stack log event before a subsequent
recoverable `pop$` underflow is rendered.

Historical capacities are observations, not Umber's safety policy. The
inventory records fixed, configured, and dynamically grown Web2C values,
including `max_strings=200000`, `hash_size=200000`,
`hash_prime=170003`, initial `buf_size=20000`, `max_cites=750`,
`max_fields=5000`, `wiz_fn_space=3000`, and `lit_stk_size=50`.
Umber exposes deterministic safe bounds for aggregate input, parser nesting,
symbols, instructions, stack/call depth, string/output bytes, diagnostics,
work, and retained cache bytes. Exact historical out-of-memory thresholds are
not a compatibility promise.

The branch ledger covers command order and recovery; identifier, literal,
brace, comment, and EOF scanning; symbol collisions and recursion; raw BIB
records; citation and crossref selection; VM lifecycle and types; stable sort;
all built-in edge categories; diagnostic/history ordering; cache and retry
identity; and the final bounded legal-program differential generator. Later
issues must refine these families into individual source branches and state
transitions without deleting or reassigning an owner invisibly.

### Bounded systematic differential gate

`tools/fixturegen/src/classic_bibtex.rs` owns the opt-in generator invoked by
`scripts/regen-fixtures.sh --area bibtex`, after that script has validated the
pinned source, merged program, Web2C configuration, and executable identity.
It is not a Cargo test and never uses an ambient BibTeX installation.

The fixed master seed is `0xB1B7EA5ED1FF0001`. It emits exactly 37 cases (one
per merged-reference built-in) and caps each generated AUX, BIB, and BST at
256, 2,048, and 4,096 bytes respectively. Every generated BST contains all
ten legal top-level commands in the declaration-to-execution order and the
same bounded two-entry database. The generator counts the following
dispatch-level merged-reference targets before execution:

| Target class                | Required |    Threshold |
| --------------------------- | -------: | -----------: |
| Top-level command branches  |       10 | 10/10 (100%) |
| Built-in dispatch branches  |       37 | 37/37 (100%) |
| Lifecycle state transitions |       10 | 10/10 (100%) |

These are source-census dispatch and lifecycle targets, not an assertion that
all internal Web2C control-flow edges have been observed. The generator must
compare unnormalized status, BBL, and BLG bytes for every case in an empty
environment with pinned lookup paths. A mismatch preserves both staged inputs,
both artifacts, status files, and a seed-bearing reproduction script under
`target/bst-differential/failures/<case>/`; the generator stops at the first
failure so that the case remains stable while it is minimized or investigated.

The completed `umber2-ild0.13.5` hardening run passes all 37 generated cases
with exact, unnormalized status, BBL, and BLG bytes. The original
`bst-diff-00` mismatch (`103` strings / `598` characters in the reference
versus `106` / `625` in Umber) is resolved by the shared Web2C string-pool
model and AUX/READ trace replay below. The same run exposed and fixed the
recoverable `stack$` and ordered `top$` log-event mismatches before reaching
full 47/47 dispatch-branch and 10/10 lifecycle-transition coverage.

### Web2C string-pool accounting

`bib-bst/src/pool.rs` owns the monotonic classic string-pool model. A pool
assigns a stable `PoolStringId` on first insertion, charges one string and its
byte length exactly once, and shares that identity across all owners; an empty
string is an ordinary chargeable value. Its explicit bootstrap sequence is the
Web2C `pre_def_certain_strings` population, rather than a summary offset.
Independent string-count and character-count limits are checked only for a
new value, so duplicate lookup never consumes capacity.

The BST compiler records its declaration, literal, integer, and internal
anonymous-function insertions as an ordered trace in the immutable compiled
style. At BLG rendering the session starts the Web2C bootstrap pool (including
the reserved `str_ptr` slot), records the top-level bare AUX name and each
opened AUX name, replays the compiler trace, then replays the immutable READ
trace. The latter records raw `@string` names and values, preambles, and the
selected declared-field values in their Web2C ownership order. This keeps
compiled-style and prepared-database caches host-neutral while preserving one
job-lifetime pool identity and exact unnormalized BLG summaries.

## VM core and built-in completion

`umber2-ild0.8` supplies the bounded execution core in
`bib-engine/src/classic_vm.rs`. It consumes immutable `bib-bst` programs and
prepared classic database state, with explicit operand and call stacks rather
than user-controlled Rust recursion. It owns command lifecycle, current-entry
state, variables and assignment, core control flow, stable `SORT`, and
detached bounded BBL/BLG effects. Fatal results retain inspection-only partial
effects while withholding publishable artifacts.

`umber2-ild0.9` completes the remaining dispatch entries in that VM: classic
string concatenation and slicing, period and whitespace rules, case and
purification handling, character conversion, name counting and formatting,
entry-type dispatch, stack diagnostics, text units, and CMR10 width values.
The fixed `entry.max$` and `global.max$` values remain visible independently of
Umber's safety limits. Focused VM coverage lives in
`bib-engine/src/classic_vm/tests.rs`; fixture-level output/diagnostic parity
continues to be owned by the later command-parity phase.

## Public execution and command boundary

`umber2-ild0.11` connects the already separate classic phases without routing
them through the Biber pipeline. `ClassicBibSession` now resolves the AUX
closure, compiles the requested BST through a bounded cache, prepares the raw
classic database through its independent cache, executes the bounded VM, and
returns detached `.bbl` and `.blg` artifacts with warning or fatal history.
Fatal VM artifacts remain in `partial_files` and are never publishable.

`bib-engine` exposes the in-process `ClassicBibCommand`; native Umber stages
only requested files into the VFS for `umber bibtex job`, then publishes the
returned artifacts. It does not invoke a system BibTeX executable. The smoke
fixture exercises this boundary both cold and cached. `umber2-ild0.21` imports
the TeX Live 2025 `plain.bst` and `apalike.bst` bytes with small, exact BBL
execution fixtures, and `bib-bst` compiles both styles hermetically.
`umber2-ild0.22` fixes the VM's quoted-assignment and control-flow operand
ordering by representing `while$` continuations explicitly rather than
draining active caller frames. `umber2-ild0.23` fixes no-comma name parsing,
and `umber2-ild0.24` makes classic name-pattern separators and abbreviated
punctuation apply per word. The imported bytes and reference BBLs must not be
replaced with normalized or compatibility-allowance tests while remaining
whole-style blockers are resolved.

The focused VM matrix covers the `plain.bst` `{ff~}` and `apalike.bst` `{f.}`
patterns, including `substring$` negative offsets. Exact public-session BBL
parity for both pinned styles is enforced against the committed fixtures:
`umber2-ild0.25` restores quoted-variable control operands and
`umber2-ild0.26` makes negative `substring$` starts count relative to the
right edge, as required by apalike's four-digit year label.

## Upstream test inventory

The classic Web2C suite consists of:

- `bibtex.test`, the `apalike.bst`/`xampl.bib` exact-output baseline;
- `bibtex-mem.test` with `memdata1.bst` through `memdata3.bst`;
- AUX include, large-author, long-line, and output-open tests.

The supplemental `bibtex-x` suite has four BibTeX8 tests and seven BibTeXu
tests. Its focused character test styles cover `add.period$`, `substring$`,
`text.length$`, `text.prefix$`, `width$`, integer/character conversion,
`num.names$`, and `format.name$`; the sort suites cover `change.case$` and
ordering. Only branches whose behavior agrees with classic 0.99d are classic
fixtures. CSF collation, UTF-8, ICU locale sorting, `is.knj.str$`, non-ASCII
job names, and other extension semantics remain explicitly extension-only and
belong to `umber2-ild0.13`. They cannot silently expand the classic identity.

## Committed fixture and regeneration

The compact `cases/smoke` reference case is intentionally small but executes
all ten BST commands. Its manifest pins the AUX, BIB, BST, command, isolated
environment, process status, reference history, BBL, BLG, and terminal byte
identities. No normalization is applied. The BLG also records the configured
capacity banner and all 37 built-in counters, providing a direct executable
census check.

The real-world corpus is deliberately separate from the compact smoke and
upstream cases. A candidate must be redistributable under a recorded license,
retain its upstream provenance/version/revision and SHA-256 identity in
`tests/corpus/bibtex/manifest.json`, run under the pinned 0.99d executable,
and add measured style-path or reference-built-in coverage. The initial
selection is the LPPL-1.3-or-later `elsarticle-num.bst` 2.1 from TeX Live 2025.
Its `elsarticle-book` case covers the style startup and book/publisher path;
its `elsarticle-article` case additionally covers article volume/page, DOI,
and URL formatting and observes `change.case$`, `substring$`, and `swap$`.
The focused `elsarticle-names` case pins multi-word `{f.~}` abbreviation
separators, while `elsarticle-month` verifies that `month = jan` resolves
through the style's visible `MACRO {jan} {"Jan."}` declaration. All four use
the public `ClassicBibCommand` and require exact status, terminal, BBL, and
BLG bytes. Candidates that expose an unresolved public-boundary parity gap are
documented as follow-up work rather than admitted with an allowance.

The only supported live-reference rewrite path is:

```bash
scripts/regen-fixtures.sh --area bibtex
```

It verifies the archive, WEB, change file, kpathsea configuration, merged
Pascal, generated C/header, Web2C configuration header, and exact executable;
builds the pinned executable when necessary; stages only the committed inputs
in a temporary directory; starts it with an empty deterministic environment;
captures status, BBL, BLG, and terminal bytes; atomically replaces changed
outputs; and runs the hermetic manifest/inventory test.

Ordinary `cargo test --tests` never reads the source checkout, invokes
BibTeX, searches an ambient TeX installation, uses a locale service, or
accesses the network. It verifies only committed JSON and fixture bytes. This
is the Phase 0 exit gate: every reference surface is owned, every initial
reference byte is content-pinned, and regeneration has one auditable route.

## Project integration

`umber2-ild0.12` adds `LatexProjectOptionsV2` and
`BibliographyProjectOptions` beside the source-compatible biblatex-only
`LatexProjectOptions`. The V2 project session accepts explicit biblatex,
explicit classic, or auto protocol selection, and sequences TeX, detection,
and bibliography resource discovery rather than speculating across stages.

Its convergence identity contains the selected backend and generated-file
content identities. Classic warning/error results publish their detached BBL
and BLG only in an accepted project generation; fatal execution and resource
or compile failure reject the candidate and retain the last accepted output.
Changing the V2 bibliography policy resets the backend session and removes
previous bibliography artifacts before the next generation, preventing a BBL
from one backend from being consumed after a switch. The WebAssembly project
binding accepts the existing biblatex object unchanged and additionally
accepts a `mode: "biblatex" | "classic" | "auto"` bibliography object; byte
and AUX/BST/BIB parsing remain inside the Rust session.
