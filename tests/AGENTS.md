# Tests Guidance

`tests/corpus` holds committed inputs and expected reference outputs for
fast differential tests.

`tests/texlive-source.lock` pins the official TeX Live 2026 source archive and
selected extracted files. `tests/conformance-texlive.lock` maps runtime files
from the authenticated hosted 2026 snapshot to their repository destinations.
`tests/native-test-assets.lock` includes those runtime records and is the
explicit SHA-256 allowlist that `scripts/provision.py worktree` copies from the
primary checkout into an isolated linked worktree. Rust tests only consume
these files; source archives and trees are symlinked from the primary checkout.

`tests/tex82-properties/` contains the generated 1,380-module pinned `tex.web` inventory and domain-local reviewed disposition/property shards. The routine `test-support` catalogue gate initializes the exact inventory to its typed deferred default, applies the shards, and validates completeness, citations, ownership, status, and exact Rust test links.

`tests/tex82-oracle-manifest.txt` pins the canonical TeX82 WEB source, ordered
Web2C portability changes, translator inputs, and repository-owned oracle
inputs. `tests/tex82-oracle/` contains the font-independent smoke program,
focused transition inputs, and the repository-owned final TeX82
instrumentation change file. Its `deterministic-clock.ch` is the system
adapter for tex.web §1337: Web2C's `onlyTeX` build ignores
`SOURCE_DATE_EPOCH`, so it installs the same pinned job clock used by hermetic
Umber worlds; `clock.tex` is the bounded clean/instrumented/repeat control.
Each split fixture's `tests/tex82-oracle/<name>-v1-semantic-matrix.txt` maps
every required TeX82 command-core observation for that fixture to its focused
input and stable final-change seam; the live oracle build consumes it as the
coverage gate. Its matching `<name>-v1-audit-matrix.txt` maps those semantic
families to exact manifest citations and useful ordinary-output channels;
hermetic validation requires bidirectional source, citation, output, family,
and committed-event coverage.
The `input-recovery.tex` program and `input-eof-*.tex` children isolate
physical-line/token delivery plus legal and scanner-status-sensitive EOF
recovery without depending on a format or fonts.
The `expansion-macros.tex` child isolates expanded delivery, TeX82 expandable
primitives, macro matching and replay, definition forms, `scan_toks`, and
direct token-list splices, with transcript-visible `\meaning`/`\show`
observations.
The `scanner-conditionals.tex` child isolates typed integer, dimension, glue,
internal, and token-list scanner results plus condition-frame lifecycle,
skipping, delimiter changes, nested evaluation, and recovery. Its EOF child
keeps incomplete skipped-text recovery focused and reference-visible.
The `case-shift.tex` child isolates TeX82 §914 `\uppercase` and
`\lowercase` code-table substitution, stored replay, and macro-definition
effects for all pinned base-engine traces.
The `alignment-delivery.tex` child isolates preamble repetition, brace-state
delivery, delimiter interception, u/v/omit templates, `\noalign`, nested
ownership, backup correction, recovery, and template retirement. Its messages
and shipped rules provide independent transcript and DVI observations.
The `geometry.tex` microfixture runs only in the isolated writable schema-v3
geometry profile. Its committed `geometry-expected.jsonl` projection pins the
finalized hpack, vpack, and shipout records together with the active source and
line without putting a full document or live reference invocation in Cargo
tests.
Acquisition and builds run only through `scripts/regen-fixtures.sh --oracle
tex82 --profile initex-eight-bit`; Cargo correctness tests never invoke the
live oracle.

`tests/etex26-oracle-manifest.txt` pins the canonical e-TeX 2.6 WEB merge,
ordered Web2C changes, translator inputs, and repository-owned inputs.
`tests/etex26-oracle/` contains the font-independent profile smoke program,
the focused base-command transition inputs, the executable semantic-event
matrix, a focused extension program and matrix covering token construction,
scanners, conditions, enquiries, sparse registers, and e-TeX state, and the
repository-owned final instrumentation change. Its primitive audit exactly
classifies every primitive declared by pinned canonical `etex.ch`;
command-core entries name extension-matrix boundaries and executor entries
name their focused parity owners. The base matrix runs in
compatibility and extended INITEX profiles and gates the complete TeX82-
applicable schema-v1 contract. The extension matrix runs against the extended
trace and rejects extension-only fragments in compatibility mode.
Clean/instrumented and repeated extension runs also compare generated effect
bytes.
Acquisition and builds run only through `scripts/regen-fixtures.sh --oracle
etex26 --profile compatibility+extended-eight-bit`; Cargo correctness tests
never invoke the live oracle.

`tests/pdftex14029-oracle-manifest.txt` pins canonical pdfTeX 1.40.29,
the ordered Web2C and configured SyncTeX changes, translator inputs, the
repository-owned shared, extension, and state final-change seams, smoke
programs, focused extension/state programs, and exact primitive audit.
`tests/pdftex14029-oracle/` contains font-independent DVI and deterministic PDF
smoke inputs, focused shared-command transition inputs and an executable
semantic-event matrix, an exact-eight-bit expansion/scanner extension matrix,
an exact cross-profile matrix that runs shared e-TeX grammar boundaries both
with and without pdfTeX's `-etex` selector, focused recovery and executor/list-
state jobs, a format-loaded saved-hyphen-code job, and a state matrix covering named parameters, font code tables,
object-independent enquiries, random/timer transitions, and committed
PDF-facing effects. The live workflow proves the 549-entry ownership
inventory, schema-validates and repeatability-checks all three traces, and
gates clean/instrumented terminal, normalized log, status, DVI/PDF, and
generated-effect transparency. Expansion-matrix rows name their exact owning
primitive. Smoke and state PDFs additionally pass an allocation-insensitive
Hayro structure projection independent of both pdfTeX variants and the command
trace. Acquisition and builds run only through `scripts/regen-fixtures.sh
--oracle pdftex14029 --profile initex-etex-eight-bit`; Cargo correctness tests
never invoke the live oracle.

`tests/pdftex-properties/catalogue.json` owns the complete retained pdfTeX
extension property mapping. It cites the pinned pdfTeX 1.40.29 source,
delegates primitive inventory authority to `docs/pdftex_primitives.md`, gives
each case one stable property ID and exact active Rust test, and dispositions
status, terminal, and log projections as pass or bug-linked xfail.
`tests/pdftex-properties/source-evidence.tsv` is the compact source-derived
module lock for that catalogue: it binds the pinned source identity and full
WEB module count to each cited module's exact title/body hash and to the
property that cites it.

`tests/oracle-regeneration-manifest.txt` pins the schema versions, exact
engine/profile selectors, source-manifest hashes, fixture areas, and expected
build identities for the three-engine regeneration interface. It also pins
each committed command-fixture selector to its exact engine, profile, manifest
path, manifest hash, executable semantic matrix, and fixture-audit matrix. The
`scripts/regen-fixtures.sh --oracle all --profile canonical` gate validates
this contract before acquisition, runs every engine's clean/instrumented
transparency workflow, and emits an aggregate uncommitted build record under
`target/oracle-regeneration/`.

`tests/corpus/distribution/cross-frontend-v1` is the closed
authored-JavaScript/Rust manifest and HTML catalogue case. Its `case.inventory`
declares every payload; `test_support::git_fixture` requires the declared,
tracked, and on-disk inventories to agree before Rust reads it, and validates
every directory ancestor without following symlinks so generated, scratch,
`target`, or alternate-checkout bytes cannot provide authority. Rust consumers
pair that Git proof with `test_support::closed_case::FixtureCase` so identity,
roles, source closure, profile, and publication metadata use the shared typed
contract. The payloads
are hand-authored contract data, not live-reference outputs, and both
`umber-distribution` and the authored JavaScript tests consume their declared
payloads from this one closed case.

`tests/corpus/bib/invocation` contains exactly three closed native `umber bib`
invocation cases: BCF success, tool mode, and invalid output-format
validation. Each directory's `case.inventory` closes its local metadata,
inputs, and exact outputs through `test_support::closed_case::FixtureCase`.
`invocation.case` schema `bib-invocation-v2` pins ordered typed literal,
declared-input, and harness-owned-output arguments, expected status,
stdout/stderr authorities, and the generated BBL/transformed-BibTeX artifact
authority. Shared source bytes are duplicated
byte-identically into the cases that consume them; no area-level input or
output remains authoritative. The owning CLI test discovers only case
directories in lexical order, executes outputs in a temporary directory, and
revalidates the closed case after execution so no ambient output can appear.

`tests/corpus/command-semantic` contains the generated V2 schema and tiny property-scoped semantic fixtures. Every `<domain>/<fixture>/` directory is a closed unit containing its `manifest.json`, conventional `<fixture>.tex` source, and each applicable `expected.<channel>` file; the directory infers domain, ID, source, ordinary file-or-empty channel dispositions, and clean status. Manifests keep projection expectations explicit and declare only channel, status, expectation, or typed capture-policy exceptions. Domain directories contain no case catalogue or shared expected-output tree. A case has two kinds of evidence, and they are not interchangeable. Its channels -- terminal, log, DVI, effects -- hold the pinned instrumented reference engine's bytes and are the correctness evidence: Umber either reproduces them (`file`), does not and pins exactly where it first diverges (`xfail`), or diverges only inside tex.web §82's error reports and is still compared everywhere else (`xfail-diagnostics`). Its `expected` is a projection in Umber's own vocabulary (`scanner:integer:2:-`, `artifact:<hash>`), which no reference engine emits, so it is derived from Umber's own canonical run and pins that behavior against silent drift rather than attesting to its correctness. Normal regeneration preserves every authored `expected` byte-for-byte and reports fresh projection differences; the correctness gate remains the authority that rejects an unexpected pass drift. Mechanical acceptance is available only for one reviewed selector at a time through `command-semantic-channels --profile PROFILE --accept-projection-change DOMAIN/CASE`; global acceptance is rejected. A case that cannot be reached by canonical execution at all is still a defect in the case rather than a permanent xfail. `alignments/` projects alignment lifecycle and packing boundaries, `conditionals/` projects conditional observations, `input-expansion/` projects filtered input and command observations with hermetic host responses, `math/` projects selected command, mode, final-box, and committed shipout boundaries, and `page-output/` projects setlanguage replay, special whatsit placement, hlist/vlist shipout artifacts, and `\leaders`/`\cleaders`/`\xleaders` placement, and `scanners-internal-quantities/` projects the internal unit probe, register fetches, glue coercion, radix forms, dimension fractions, and scaled division. The generic runner in `tex_command_stream::semantic` discovers every fixture without a Rust registry, validates catalogue ownership, source bounds, exact provenance, duplicates, strict xfail fingerprints, closed local inventories, and regular-file locality, then compares concise canonical-main-control projections. It lives in the library rather than the test binary so `scripts/regen-fixtures.sh --area command-semantic` drives the same code the gate does; `tools/tex-command-stream/tests/it/command_semantic.rs` holds only the assertions. Cargo tests invoke no live TeX and never read the long-document trace registry.

Each command-semantic manifest carries typed capture policy. Regeneration
selects the requested profile directly from the validated V2 cases, builds one
genuine format through `-ini`/`\dump`, and captures only the subsequent `-fmt`
jobs. The `raw-tex82-loaded` policy selects exactly 176 jobs: the reviewed
35-case scanner/input/conditional cohort, 55 ordinary main-control cases, and
all 18 alignment, 34 math, and 33 page-output cases, plus the bounded
line-breaking paragraph-shape case.
The latter excludes `hyphenation-data` by explicit case-local policy and excludes
`hyphenation-errors` and
`final-cleanup-end-or-dump`.

Each case also declares a `channels` block accounting for both bounded contracts -- `events` and `status` at the authored-fragment root-EOF boundary, plus `terminal`, `log`, `dvi`, and `effects` from a complete TeX job -- because a projection asserts one observable and is not coverage of either contract. Real pdfTeX exposes no host-fragment boundary: after the same source reaches root EOF it necessarily enters TeX82 §360. The generic runner therefore executes the identical canonical driver, profile, source, and host inputs twice, changing only `RootCompletionPolicy`; it rejects the pair if their observations diverge before the typed termination boundary. This guard keeps the fragment projection and complete-job byte oracle as explicit evidence contracts rather than competing engine authorities. A case with no block fails validation; the only exemption is a case whose run does not complete, and it is granted only to a case already pinned as `xfail`. **The authority rule above governs `expected` (the projection), and the same rule now governs the channel bytes too.** Every applicable fixture-local `expected.<channel>` file holds the pinned instrumented pdfTeX 1.40.29 oracle's own bytes, for `file` and `xfail` alike (`umber2-alfh.1`/`umber2-alfh.7`): a channel where Umber does not yet match those bytes is `xfail` with a `mismatch` pinning the first divergence and a `bug` -- or `xfail-diagnostics` with a `bug` alone, when the divergence is confined to §82's error reports and the rest of the channel still matches -- never a self-pin against Umber's own output. `StreamDisposition` carries no `authority` field, because there is now exactly one place a committed channel's bytes can have come from; reading a green `file` channel as canonical evidence needs no further check.

A `mismatch` records only _where_ a channel first diverges, which is enough to
tell that a case is wrong and never enough to tell what to change. To see both
sides in full, run
`cargo run -p tex-command-stream --bin command-semantic-channels -- --diff <substring>`:
it prints each matching case's source and then the oracle's terminal text
beside Umber's, line-numbered, with differing rows marked and spaces shown as
`·` (§314's descriptors end in a load-bearing space). `--diff-log` shows the
transcript instead, which is where §90 puts an error's help lines, so a
help-routing difference is invisible in the terminal one. Neither writes
anything, so both are safe against an uncommitted tree.

The `etex-diagnostics/` domain owns bounded e-TeX-only diagnostic command
microfixtures. Its sessions explicitly install the e-TeX INITEX profile and
project detached effects, selected unchanged state, and pinned e-TeX/SyncTeX eqtb register selectors.

The `input-expansion/` domain's e-TeX outer-validity EOF case pins the e-TeX
observer's argument-free §336 diagnostic separately from the TeX82 profile's
scanner-status-bearing seam.

The `input-expansion` domain owns the thirteen audited input and expansion semantic tiers: ten exact passes plus three strict xfails linked to their existing canonical defect beads.

`tests/corpus/command` contains committed canonical command-core fixtures.
Each engine/fixture directory carries a canonical contract-v1 `manifest.json`,
focused INITEX sources, a manifest-bound normalized schema-v1 `events.jsonl`,
and exact ordinary artifact observations. Cargo tests load these files
hermetically through `tex_oracle::CommittedFixture`; live reference generation
uses only the exact `--oracle`, `--profile`, and `--fixture` selector pinned in
`tests/oracle-regeneration-manifest.txt`. See
`docs/command_semantic_fixtures.md`.

`tests/latex-source.lock` pins the common TeX Live 2026 files plus
mode-specific repository-local format configuration inputs, byte lengths, and
SHA-256 identities opened while building the Umber-native `latex.fmt` and
`pdflatex.fmt`. Common `source`/`local` records apply to both modes;
`pdflatex-source`/`pdflatex-local` records extend only the PDF format closure.
The explicit LaTeX format builder verifies the selected closure before every
build. With `--publish-input-closure`, it also emits the canonical request-key
closure and construction-input identities consumed by the schema-3 TeX Live
snapshot publisher. Publication rejects a format whose input identity differs
from the runtime basename winner; LaTeX has 61
keys and pdfLaTeX has those same keys plus its three mode-specific records.
`tests/latex/language.dat` keeps the format's English language slot and
hyphenation minima deterministic without depending on generated TeX Live
`texmf-var` state. Its `usenglish`, `USenglish`, and `american` synonyms are
the complete upstream US English alias closure required by the paired Babel
runtime. Both source-loaded and frozen-format equivalence fixtures assert that
the aliases select the English slot and exercise Babel's `USenglish` option.

`tests/texlive-snapshot.lock` pins the complete publisher-visible runtime-tree
digest for the 2026-03-01 snapshot plus compatibility-critical LaTeX kernel,
latex-dev `array.sty` v2.7a, and generated pdfTeX map identities. The production
snapshot builder verifies this lock before publication; changing a snapshot
requires an explicit lock and distribution-identity update, never reuse of a
mutable TeX Live year directory.

`tests/latex-parity-manifest.txt` pins one complete official LaTeX2e repository
archive rather than individual support or test files. Setup derives the DVI
cohort from same-stem standard `.tlg` shipout markers in the declared package
scopes, then the live reference run distinguishes actual classic-LaTeX DVI
cases from alternate test configurations. Explicit manifest `skip` records
must name a concrete unsupported engine layer and remain visible in the census
summary; they are not expected-failure aliases for TeX/e-TeX/LaTeX bugs.
Exact `non_dvi` records pin every alternate configuration by path; any
unmarked case must emit a reference DVI, and skipped cases must first be
verified as members of that reference-DVI cohort.
`scripts/setup-latex-parity-tests.sh`
owns acquisition under gitignored `third_party/latex2e-parity/`; do not copy or
modify individual upstream LPPL files in the repository.

`tests/latex/` contains committed, compact LaTeX-DVI inputs used by explicit
format and corpus gates. `firstaid-coherence.tex` enumerates the kernel-side
controls captured by the pinned first-aid hooks and is loaded by both format
equivalence fixtures; package-owned controls are checked only after their
owning package loads. `format-equivalence.tex` must remain deterministic
and is run both from source-initialized kernel state and the serialized format.
`pdflatex-smoke.tex` provides the corresponding font-independent PDF format
equivalence gate, while `pdftexconfig.tex` pins pdfLaTeX's output policy without
depending on mutable TeX Live configuration state.
`pdflatex-representative.lock` records the complete authenticated positive
runtime closure shared by the pdfLaTeX source-initialized and loaded-format
representatives. Its source records retain authoritative virtual paths, byte
lengths, and SHA-256 identities; construction-only keys remain owned by
`tests/latex-source.lock` and local document/AUX inputs are excluded.
The four base-class documents are run for three clean passes by
`scripts/check-latex-corpus.sh`, which requires banner-normalized byte-identical
DVI and exact `.aux`/`.toc`/`.lof`/`.lot`/`.out` file parity with TeX Live 2026.
Their union of format-loaded TeX/TFM inputs must exactly match
`latex-runtime.lock`; `scripts/build-wasm-latex-bundle.sh` publishes that same
typed closure and the generated native format through the WASM manifest.
`scripts/check-latex-parity.sh` is a separate live-reference tier: it restores
one pregenerated `latex.fmt` into clean per-case sessions and gates only
preamble-comment-normalized, otherwise byte-identical DVI.

## Corpus Layout

The bounded execution minifixture families `exec`, `etex_exec`, `typeset`,
`math`, `align`, `tex_exec`, and `expand`, plus `lexer`,
`lexer_dynamic`, `stabilization`, and `canonical-dvi`, use closed
self-contained case directories:

```text
tests/corpus/<area>/<case>/
  <case>.tex or source.tex
  expected.<channel>
  <case-local support inputs>
```

Every exact runtime input and applicable committed output belongs in that
directory. The four lexical/session families use `source.tex`, and
`stabilization` cases are source-only. There are no area-level support files or expected-output trees for
these families. Discovery is lexicographic by case name. The `test-support`
inventory gate equates the filesystem with Git's tracked regular-file
inventory and rejects non-directory cases, missing sources, untracked or
ignored additions, symlinks, non-regular files, and authorities that resolve
outside the selected checkout.

`tests/corpus/exec` contains fast execution-core parity cases. These compare
`umber run`'s terminal output with committed normalized reference terminal
fixtures through the shared `test_support::normalize::exec_log` helper. The
normalizer retains TeX82 §§310--318 `show_context` headers and their indented
continuation lines, so the committed terminal fixtures cover complete live input
context rather than treating it as incidental framing. The
manual parity test reads the committed `expected.terminal` files; regenerate them through
`scripts/regen-fixtures.sh` when reference output intentionally changes. Live
reference regeneration uses `-ini` plus the seven printable catcodes installed
by `umber run`, matching its fresh INITEX parameter state rather than
inheriting Plain's format assignments. It captures pdfTeX's terminal channel,
not its transcript file: canonical TeX deliberately emits some diagnostic
newlines to only one of those channels.
Only `math_component_recovery` remains; migrated duplicates, including
`paragraph_line_shape`, were retired into property-scoped command-semantic cases. See
`docs/golden_corpus_dispositions.md` for the exact ownership accounting.

`tests/corpus/etex_exec` contains extension-mode e-TeX diagnostic parity
cases. The `umber` runner supplies `--etex`, while fixture regeneration uses
the live e-TeX-capable reference executable with `-ini -etex` plus the same
seven printable catcodes. Small `<case>.txt` case-local inputs
are copied into the reference run directory for `\readline` coverage.

`tests/corpus/typeset` contains fast box/list dump parity cases for the
typesetting layer. These compare `umber run --show-fixtures` terminal output
with committed normalized reference terminal fixtures through the shared
`test_support::normalize::box_dump` helper; that helper uses the same
diagnostic-log normalizer as `exec_log` and likewise retains §§310--318 context,
including indented continuation lines.
Reference regeneration uses `-ini`
plus `umber run`'s printable catcodes, so TeX82 §660-§675 box diagnostics
are compared under the same parameters and on the same output channel; they
are not normalized away. In this mode, `umber` writes only the
collected terminal/log diagnostic text to stdout and skips the CLI's extra
final `World` effect commit. TeX shipouts still commit their own effect prefix,
so stream whatsits shipped by `\shipout` or final cleanup can materialize
output files even under `--show-fixtures`; pending immediate stream effects do
not materialize because their final commit is skipped.
The retained thirteen-case census is pinned by `test-support`; migrated box,
font, and scalar duplicates were retired. See
`docs/golden_corpus_dispositions.md` for the tier rationale.

`tests/corpus/dvi` and `tests/corpus/page` are retired (`umber2-alfh.3`). Their
thirty-two sources are now thirty-one `command-semantic` cases -- one source
was byte-identical in both areas -- placed in `page-output`, `math`, and
`alignments` by what each exercises. The reason to move them is that a
`.expected.dvi` fixture pinned one channel against Umber's own prior output,
while a minifixture compares terminal, log, DVI, and effects against the
pinned oracle. `tests/corpus/math` and `tests/corpus/align` remain on the
`.expected.dvi` path and are regenerated by `scripts/regen-fixtures.sh`, which
copies each source plus pinned CM TFMs and case-local support files into a temporary
run directory, runs `tools/refexec` against the live reference engine, and
rewrites `expected.dvi` only when the preamble-comment-only DVI comparison
detects a real byte change.

`tests/corpus/canonical-dvi` holds two closed `source.tex`/`expected.dvi` pairs that
the canonical-divergence regression tests in
`crates/umber/tests/it/e2e_conformance.rs` read. It is deliberately not a
regenerated DVI area: those tests pin a specific past divergence, so the
fixtures must not track the reference engine.

`tests/corpus/stabilization` contains two closed, source-only generated-input
fixed-point cases shared by native and WebAssembly conformance tests. These sources use
only engine primitives and repository-owned TFM data; the LaTeX-surface case
defines its compact document-command vocabulary in the source so the default
test tier does not depend on a live TeX installation or a generated format.

`tests/corpus/tex_exec` contains twenty-nine small reference-observation sources and
normalized `expected.ref` outputs. The active TeX82 cohort is executed by
`crates/tex-exec/tests/fixture_parity.rs`, which compares exact ordered
normalized terminal/log lines for every case with both the preserved reference
bytes and bounded canonical `MainControl` output, and requires clean `\end`
completion. These files predate a reproducible pinned capture contract, so
`scripts/regen-fixtures.sh --area tex_exec` validates the active consumers but
never rewrites `expected.ref`. New oracle-backed executor coverage belongs in
the command-semantic or pdfTeX-extension tiers. The `pdf_output_policy`,
`pdf_image_config`, `pdf_metadata_config`, `pdf_font_config`, and
`pdf_microtype_effects` observations were captured in INITEX mode so PDF parameter defaults, grouping, first-write
recovery, font diagnostics, and effective microtype nodes do not inherit
format-file assignments; their fixtures anchor the corresponding Umber policy
tests. The seven `pdf_navigation_*` cases and `pdf_ximage_enquiries` moved
without byte changes to closed cases under `tests/pdftex-properties/fixtures`.
Their active pdfTeX extension runner dispositions every status, terminal, and
log observation as pass or strict bug-linked xfail. The enquiry case uses
deterministic fixturegen-owned PNG, JPEG, and typed `pdf_writer` three-page PDF
inputs without committing opaque binary support assets.

The retired `tests/corpus/tex_exec_io` cases are owned by the oracle-backed
command-semantic fixtures `page-output/closeout-stream-selectors`,
`page-output/open-close-without-write`, and
`page-output/top-open-write-close`. Their all-channel evidence includes the
generated artifact bytes that the legacy effects/output summaries projected.

`tests/corpus/math` contains primitive-only math DVI parity fixtures plus
committed `expected.dvi` reference fixtures. Each case carries its exact
`math_preamble.inc`; keep that include free of `plain.tex` dependencies and
keep individual `.tex` cases small. The cargo test runs each case against its
committed DVI fixture; the regeneration path runs the reference engine in
INITEX mode for this area, copies the shared include beside each case, and
pins `cmr10`, `cmmi10`, `cmsy10`, and `cmex10` TFMs so text/script/
scriptscript family selection observes the same metrics as Umber.

`tests/corpus/align` contains alignment-focused DVI parity fixtures for
`\halign`, `\valign`, spans, omission, `\noalign`, nested alignment, and
display alignment, with committed `expected.dvi` reference fixtures.
The cargo test runs each case against its committed DVI fixture, and
`scripts/regen-fixtures.sh` runs the same area with the same pinned CM TFMs as
the other DVI corpora; keep cases primitive-only.

`tests/corpus/pdf` contains 15 closed primitive-only minimal PDF case
directories. Each co-locates its `source.tex`, pinned pdfTeX reference PDF,
exact Umber PDF, canonical structure projections, grayscale PGM renders,
renderer/hash attestations, and every exact font, encoding, PK, or included-PDF
input it opens. Regeneration uses pdfTeX 1.40.29
and Poppler `pdftoppm` 25.08.0 only through `scripts/regen-fixtures.sh`; cargo
tests remain hermetic by rebuilding Umber bytes and checking the committed
structure, byte, render, and digest chain without invoking either tool. The
`embedded_subset_controls_negative` case pins nonpositive ToUnicode generation
and signed-nonzero Type-1 CharSet omission. The `pk_bitmap_300` and
`pk_bitmap_600` cases stage the committed `cmr10.<dpi>pk` programs and prove
resolution-dependent Type3 font output. PK assets are exact-name runtime
resources; fixture regeneration may locate and pin a missing asset, but Umber
must never invoke a font generator or ambient fallback.
The `embedded_tagged_spacing` case pins ordered fake-space controls, a custom
fallback font selection, physical ligature output, nested font changes, and
cross-page state together with exact Poppler extraction and raster parity.
The `annotations_running` case pins general annotation geometry, two-page
running-link continuation, per-page link margins, unique annotation ownership,
and `/Annots` encounter order against pdfTeX 1.40.29.
The `form_xobjects` case pins nested Form XObject dictionaries and decoded
streams, attributes/resources, h/v/math placement, reuse, saved positions and
form-local snapping. Its exact-byte and retained-session replay gates consume
the same committed source.
The `navigation_structures` case pins the composed multipage destination name
tree, outline hierarchy, thread/bead graph, deterministic bytes, and unchanged
page pixels.

`tests/corpus/bib/upstream-2.22` contains the verbatim redistributable test
data from the pinned bibliography compatibility baseline plus its upstream
Artistic-2.0 license. `manifest.json` records the full upstream commit, byte
length, and SHA-256 of every imported file. The `bib-engine` integration test
verifies that the manifest names exactly the committed import, with no missing
or extra bytes. Regenerate the complete import only with
`scripts/regen-fixtures.sh --area bib` and a local upstream clone selected by
`UMBER_REF_BIBER_SOURCE`; ordinary Cargo tests never run the reference program.

`tests/corpus/bib/invocation` contains small native `umber bib` inputs and
exact expected terminal/error byte fixtures. These exercise the in-process
command adapter and must not invoke the live reference implementation.

`tests/corpus/bibtex` contains the classic BibTeX 0.99d Web2C compatibility
inventory, source/configuration/executable manifest, and nine closed cases.
Every case co-locates its AUX, BIB, BST, applicable BBL/BLG/terminal bytes,
`case.json` provenance and identities, and `case.inventory`; shared styles are
duplicated byte-identically and are never family-level payload authority.
The root JSON files are implementation coverage and provenance inventories,
not payload loaders. Consumers validate `ClosedCase` again on every access.
The compact smoke style executes all ten BST commands; the BLG enumerates all
37 built-ins. Regenerate only with
`scripts/regen-fixtures.sh --area bibtex`, which builds and verifies the pinned
merged `bibtex.web` plus `bibtex.ch` executable before running it in an empty
environment.

`tests/corpus/e2e` receives gitignored final-DVI oracles for Story, Gentle,
TRIP, and e-TRIP. Their Cargo integration tests run Umber directly in process
and FAIL, naming every missing file and its materialization command, when an
external input or local oracle is absent; they never return cleanly over an
unexecuted byte-exact comparison. The gitignored entries in the repository
`.gitignore` are the single source that the gate registry in
`crates/umber/tests/it/e2e_conformance/assets.rs` and the
`scripts/check-and-test.sh` preflight both bind to; adding a fifth oracle
requires registering a fifth gate. See "End-to-End Conformance Gate Contract"
in `docs/testing_infrastructure.md`. Run
`python3 scripts/provision.py worktree .` in the primary checkout to acquire
the pinned third-party inputs and generate all four oracles with the
instrumented pdfTeX 1.40.29 build. The
script delegates regeneration to `scripts/regen-fixtures.sh`; TRIP uses its
two-phase pdfTeX workload and never copies `third_party/trip/trip.dvi`.
e-TRIP reuses the pinned `trip.tfm` directly and requires exact DVI parity;
the same gate also compares the pinned official V2 INITEX log, loaded log,
terminal photo, DVItype projection, and exact generated output under the
bounded contract in `docs/trip.md`.

Other, non-directory corpus families retain the flat
`<case>.expected.<kind>` convention.

## Fixture Updates

Use `test_support::assert_matches_fixture(area, case, kind, actual)` for
fixture assertions. When output changes intentionally, regenerate committed
fixtures only through:

```bash
scripts/regen-fixtures.sh --incremental
scripts/regen-fixtures.sh --all
scripts/regen-fixtures.sh --area exec
scripts/regen-fixtures.sh --case dvi/single_glyph
```

Modes:

- `--incremental` looks at changed paths under `tests/corpus` and regenerates
  only the affected cases or areas.
- `--all` regenerates all committed fixture areas.
- `--area AREA` regenerates one area, such as `lexer`, `expand`,
  `lexer_dynamic`, `exec`, `typeset`, `dvi`, `page`, `math`, `align`, `pdf`,
  `e2e`, `bib`, `etex_exec`, or `tex_exec`.
- `--area bib` re-exports the pinned upstream bibliography test data directly
  from its fixed Git commit, rebuilds its SHA-256 manifest, and validates the
  hermetic `bib-engine` integration test.
- `--case AREA/CASE` regenerates one case exactly for text/native and DVI
  areas.

The script requires a live reference TeX (`pdftex` or `tex` on `PATH`, or
`UMBER_REF_TEX=/absolute/path/to/pdftex`) for reference-derived text and DVI
fixtures. Text/native regeneration builds and runs `tools/fixturegen`; DVI
regeneration builds and runs `tools/refexec`, copies the pinned `cmr10`,
`cmmi10`, `cmsy10`, and `cmex10` TFMs from
`crates/tex-fonts/tests/fixtures/cm`, copies area-local support files such as
`math_preamble.inc`, and validates the affected cargo test after rewriting.
It writes raw reference DVI bytes and uses only the existing DVI preamble
comment normalization to decide whether an existing fixture is unchanged.
`tex_exec` is the exception: its preserved `expected.ref` files have no pinned
generator, so its area and case commands are validation-only and require no
live reference executable.

`scripts/regen-fixtures.sh --area fonts` runs the live `tftopl` font
cross-check. It requires `tftopl` on `PATH` or
`UMBER_REF_TFTOPL=/absolute/path/to/tftopl` and does not rewrite fixtures.

## Cargo Test Scope

`cargo test --workspace --tests` is the correctness gate. It reads committed
fixtures and must run without TeX tools on `PATH`; keep warmed
`cargo test --tests` under the documented 10-second target in
`docs/testing_infrastructure.md`. `scripts/check.sh` is the broader local
quality gate that includes formatting and clippy.

Font metric parity tests use a locked local TFM corpus under
`third_party/fonts/`, which is gitignored. Populate it from the authenticated
TeX Live 2026 runtime with:

```bash
python3 scripts/provision.py worktree .
```

The live `tftopl` corpus cross-check runs through
`scripts/regen-fixtures.sh --area fonts`. When optional third-party font files
are absent in that explicit mode, it prints a clear skip message for that
corpus while still checking committed edge fixtures.

Long-running document corpus parity uses the line-oriented
`tests/corpus-manifest.txt` file for external TeX documents that are fetched
rather than committed. Acquire and verify the corpus, the required TeX Live
support files, and all local end-to-end DVI oracles with:

```bash
python3 scripts/provision.py worktree .
```

Setup builds `tools/fixturegen` and runs `--sync-corpus`, writing exact fetched support
inputs and documents to gitignored `third_party/corpus/`, verifies the manifest
SHA-256 values, and fails clearly on cached or fetched hash drift. Manifest
entries use `key value` lines; repeated `url` fields are ordered locators, and
each downloaded candidate must match the entry digest before acceptance.
Support entries record provenance and licensing;
document entries additionally select a `format_source` and record the reference
DVI SHA-256 after the same banner-only normalization used by `tools/refexec`.
Fixture regeneration pins `SOURCE_DATE_EPOCH=1783604160` and
`FORCE_SOURCE_DATE=1` so reference TeX and Umber observe the same clock when
external documents write date primitives into the DVI body. After setup, the
Cargo conformance tests consume only local files and require no network access.

Run the fixture-backed end-to-end DVI conformance tests explicitly with:

```bash
cargo test -p umber --test it e2e_conformance_story -- --nocapture
cargo test -p umber --test it e2e_conformance_gentle -- --nocapture
cargo test -p umber --test it e2e_conformance_trip_canonical -- --ignored --nocapture
cargo test -p umber --test it e2e_conformance_etrip -- --nocapture
```

For each Story and Gentle test, the shared Rust harness stages the selected real
`format_source`, document, `third_party/hyphen/hyphen.tex`, and all TFM files
loaded by Plain. Cargo tests run only Umber and compare its final output with
the local oracle. Live reference TeX is used only by
`scripts/regen-fixtures.sh`; regeneration verifies `expected_ref_dvi_sha256`
before updating the oracle. The generated e2e `.expected.dvi` files are
licensing-sensitive derivatives and must remain gitignored and uncommitted.
Reference drift, Umber failures, and byte mismatches
write automatic triage bundles under `target/conformance-triage/<doc-name>/`
containing byte context, page-limited dvitype-style disassemblies and diff,
tracing-output logs, and a summary naming the divergent page and opcode when
available. The `cargo test -p parity-harness self_test_bundle_pinpoints_page_and_opcode`
command runs the Rust harness's synthetic fast bundle check; it intentionally
changes one DVI movement opcode and verifies the summary pinpoints the page and
opcode without using the external corpus.

The official Knuth TeX82 TRIP and e-TeX V2 e-TRIP conformance materials are
pinned separately in `tests/trip-manifest.txt`. They are fetched into
gitignored `third_party/trip/` by `scripts/provision.py worktree`; do not
commit the fetched CTAN files. The registered Cargo gate fails with
materialization instructions when a required source, TFM, or oracle is absent.
The ignored `e2e_conformance_trip_canonical` direct-profile probe and
`e2e_conformance_etrip` Cargo integration test share an in-process two-phase
format-create/format-load helper. Successful recipe-owned dump construction
gates exact command channels plus structured publication and reload invariants,
retains advisory geometry reports, and excludes implementation-internal terminal and log
diagnostics. Loaded execution retains exact terminal and log comparison, except
that complete TeX82 §638 shipout memory-usage records are typed
allocator-accounting advisories while all surrounding text remains byte-exact,
and applies
the same preamble-comment-only, byte-identical final-DVI assertion used by
Story and Gentle. They never invoke an Umber subprocess.
Run TRIP with
`cargo test -p umber --test it e2e_conformance_trip_canonical -- --ignored --nocapture`.
Run the required official e-TRIP artifact gate with
`cargo test -p umber --test it e2e_conformance_etrip -- --nocapture`.

## Proptest Budgets

Replay-identity proptests use `PROPTEST_CASES` for their case budget. Leave
the default small enough for `cargo test --workspace --tests`; raise it for
local long runs, for example:

```bash
PROPTEST_CASES=1000 cargo test -p umber --test it replay_identity
cargo test -p umber --features shadow --test it replay_identity
```

Effectful rollback/commit fuzzing uses the same budget variable and is wired
through:

```bash
scripts/effectful-rollback-fuzz.sh
PROPTEST_CASES=1000 scripts/effectful-rollback-fuzz.sh
```

The script defaults to 10,000 generated cases and covers World effects,
pre-commit leak assertions, rollback state-hash identity, and committed-prefix
replay checks. Do not move that long run into default cargo tests.
