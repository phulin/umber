# Recent arXiv PDF and DVI Corpus

## PDF corpus contract

PDF is the primary comparison output for the dated-source corpus. Both engines
compile the complete unchanged archive with its declared LaTeX/pdfLaTeX format
and selected TeX Live runtime, with PDF output explicitly selected. XeLaTeX
remains unsupported. DVI remains an explicit diagnostic mode; its existing
captures are preserved and are not reused as PDF evidence.

Reference qualification requires successful exit and a complete PDF. Every
qualified paper is attempted by Umber, even after another paper differs or
fails. Results distinguish reference failure, Umber failure, PDF projection
difference, comparator error, and projection equality. Output mode and comparator
identity belong to the run identity and are checked on resume and verification.

Independent papers run concurrently within each phase. `--jobs` defaults to
available CPU and memory capacity with headroom for orchestration; `--jobs 1`
selects serial diagnosis. Reference qualification finishes before the Umber
phase starts. Each paper keeps its own working directory and execution limits,
and completion messages appear as papers finish. Summary ordering follows the
source lock. An interrupted parallel run may have gaps between completed rows;
resume validates and reuses those rows, while verification cannot report a
complete pass for missing work. Changing `--jobs` does not change the TeX input
or binary identity of a saved result.

The PDF comparator uses the independent Hayro parser and a bounded corpus
graph/content projection, with page geometry and decoded content included.
Shared objects are visited once and large stream payloads are hashed rather
than expanded into printable hex. Inline images retain dictionary and sample
evidence. Object numbers, compression, and file layout are not page semantics. Text, positions,
resources, images, and document structure remain comparison evidence. Equality
in this lane means equality of that declared projection, not a claim of pixel
identity or complete PDF specification validation. External rendering and
validation remain separate consumer checks described in
[PDF test architecture](pdf_test_architecture.md).

Build the comparator with `cargo build --profile test -p test-support --bin
pdf-compare`, then run the prepared corpus:

```sh
python3 scripts/run-arxiv-texlive.py \
  --source-lock scripts/pdftex-arxiv-recent-sample-100.lock.tsv \
  --archives target/parity-wave/arxiv-acquisition \
  --preparation target/arxiv-pdf-formats/preparation.json \
  --umber target/arxiv-pdf-utf8/umber \
  --pdf-comparator target/debug/pdf-compare \
  --results target/arxiv-pdf-corpus
```

PDF is the default. `--output-format dvi --parity-harness PATH` selects the
DVI comparator instead. Use a new results directory when changing modes,
formats, or binaries. `--verify-only` rechecks the saved evidence without
compiling papers; `--qualify-only` records reference results before comparison.
The comparator emits `equal`, `different`, or `error`, with input identities,
page counts, projection hashes, and the first differing projection line.
Decoded content hashes are additional evidence, not an equality requirement:
PDF whitespace may differ while the decoded operations agree.

An independent consumer pass distinguishes structural differences from visible
or text-extraction differences. Install PyMuPDF in a local environment and run:

```sh
uv venv target/arxiv-pdf-consumer-env
uv pip install --python target/arxiv-pdf-consumer-env/bin/python pymupdf
target/arxiv-pdf-consumer-env/bin/python scripts/compare-arxiv-pdf-render.py \
  --results target/arxiv-pdf-corpus --output target/arxiv-pdf-render
```

This compares RGB pages at 144 dpi with annotations enabled, page geometry,
and extracted text. It records consumer versions, authenticated input hashes,
per-page hashes, and separate raster/text outcomes. No tolerance hides changed
pixels. Pixel equality at this resolution is a consumer result, not proof of
equality at every resolution. Structural and consumer results remain separate;
font subset encodings can differ while pages render identically. Each paper
has the same 120-second and 1,536 MiB process limits. Rendering uses bounded
parallel workers selected from host CPU and memory capacity; `--jobs N`
overrides that choice, and `--jobs 1` runs serially. Each consumer process
installs its own memory limit before opening PDFs. The consumer requires the
complete reference inventory recorded by the corpus summary, so a missing row
cannot turn a subset into a pass. Reference failures and unsupported engines
are counted as ineligible. A
reference-qualified paper without a successful Umber PDF is unavailable;
repaired or unreadable PDFs are errors. The consumer verdict passes only when
every reference-qualified paper matches both channels. Structural differences
remain diagnostics: direct versus indirect arrays and equivalent font-subset
representations may differ without changing rendered pages or extracted text.

Normal pdfTeX formats must not enable encTeX. The selected TeX Live
`fmtutil.cnf` uses extended pdfTeX and `cp227.tcx` for LaTeX, without `-enc`.
LaTeX tests whether `\mubyte` exists before installing its normal UTF-8 input
handling; advertising an unimplemented encTeX capability changes package
semantics. Reference and native formats must use the same standard profile.

## Dated-source PDF baseline

The September 2026 capture uses rebuilt, standard-profile LaTeX and pdfLaTeX
formats for all four selected TeX Live years. Reference qualification in
`target/arxiv-pdf-corpus/` succeeds for all 94 declared-pdfLaTeX papers,
covering 2,266 pages. The two declared-LaTeX papers (`2606.27112` and
`2607.10883`) need EPS conversion through `repstopdf`, which is unavailable in
this environment. The four XeLaTeX papers remain unsupported. Neither the
paper sources nor the ordinary execution limits were changed.

The completed serial baseline produces 82 Umber PDFs and 12 Umber failures,
with no comparator errors or pending papers. All 82 pairs differ under the
strict graph projection. Their reference PDFs contain 1,519 pages and their
Umber PDFs contain 1,521: `2606.29843` grows from 10 to 11 pages and
`2606.04385` from 13 to 14. Structural differences alone do not establish the
size or visibility of an output change; use the consumer results below.

`target/arxiv-pdf-render/summary.json` records the independent consumer pass
across all 100 rows, using the same PyMuPDF 1.28.2 consumer as the release
checks. All 82 PDF pairs were compared, with no consumer errors. Its verdict
is `FAIL`: 47 of the 94 reference-qualified papers match both required
channels, while 35 differ and 12 have no successful Umber PDF.

| Consumer result                            | Papers |
| ------------------------------------------ | -----: |
| Exact pixels and extracted text            |     47 |
| Extracted text matches; pixels differ      |     21 |
| Both pixels and extracted text differ      |     14 |
| Reference qualified; Umber PDF unavailable |     12 |
| Reference ineligible or unsupported engine |      6 |

The pixel-only differences are not uniformly harmless: some are single-pixel
rounding effects, while others affect substantial figure regions. Per-page
hashes and the first differing page's changed-pixel count, bounds, and channel
delta provide triage evidence without silently applying a tolerance.

| Umber failure class                      | Papers |
| ---------------------------------------- | -----: |
| Stale command delivery                   |      6 |
| Two-minute execution timeout             |      2 |
| Checkpoint release validation            |      1 |
| Compressed indirect PDF number import    |      1 |
| Missing PDF tagging object during import |      1 |
| Interlaced PNG alpha import              |      1 |

The eight one-page representative documents in
`target/arxiv-pdf-representatives/` all compile with both engines. The
independent consumer capture in `target/arxiv-pdf-representatives-render-v1/`
reports eight pixel-exact and extracted-text-exact matches using PyMuPDF 1.28.2.
This covers LaTeX and pdfLaTeX format loading for 2023–2026, not substantial
real-paper coverage for years absent from this sample. The strict graph
comparator reports structural differences for these same eight pairs; for
example, an indirect font-width array and an equivalent inline array have
different graph projections but identical consumer results.

A second eight-document run in `target/arxiv-pdf-representatives-parallel/`
uses the automatic worker policy (11 available workers on this machine).
All eight references and all eight Umber compilations succeed;
`target/arxiv-pdf-representatives-parallel-render/summary.json` again reports
eight exact pixel/text matches. The full 100-paper baseline above was captured
serially, before parallel scheduling was integrated.

## PDF parity repairs

The bounded parallel repair capture at `target/arxiv-pdf-wave2-font-rotation/`
uses the same 100 archives, selected distributions, standard formats, and
ordinary execution limits. At commit `0252a4e96`, 93 of the 94
reference-qualified papers produce Umber PDFs. Its independent consumer pass
in `target/arxiv-pdf-wave2-font-rotation-render/summary.json` reports 72 exact
pixel/text matches, 16 pixel-only differences, five differences in both
channels, and one unavailable PDF. All 47 baseline matches remain exact;
there are no consumer errors. The six reference-ineligible rows are unchanged.
All 93 completed pairs have matching page counts, totaling 1,678 pages per
engine. The strict graph projection still differs for every completed pair.

The repairs address shared engine behavior, without changing paper sources
or tolerating different pixels:

- Retained command delivery is readmitted before the executor hands off a
  command at a character-run boundary. This fixes all six stale-delivery
  failures, including the reduced LaTeX last-page hook case.
- Releasing a checkpoint validates its owner and cursor without requiring the
  restore-only condition that every TeX group has closed. TeX permits normal
  termination inside an open group. The 2023 paper `2606.28801` now matches
  all 13 pages exactly.
- PDF import resolves objects through the verified cross-reference index,
  including compressed numeric objects. It no longer mistakes the suffix of
  object `111` for object `11`, or searches compressed numbers in the original
  file bytes. Missing required objects remain errors.
- PNG import reconstructs Adam7 passes before separating color and alpha,
  with bounded decoded size and the existing sample-depth policy.
- PDF origin translations use canonical scaled arithmetic and printed
  precision. This removes the small placement differences in `2605.27003`
  and `2606.04749`, among others.
- JPEG natural dimensions use JFIF/Exif density with pdfTeX's marker
  precedence and rounding. The 300-dpi figure in `2606.29843` now has its
  correct size; the document returns from 11 pages to 10 and matches exactly.
- Local TeX and image lookup tries exact filenames before the selected
  runtime's case-insensitive filename fallback. Input audits authenticate the
  physical archive member. This restores omitted figures in `2607.04563`
  and resolves the extra-page difference in `2606.04385`.
- Type-1 font-map slant and extension are applied before subsetting. This
  restores synthetic italic headers and makes all three affected papers
  (`2606.26320`, `2606.26813`, and `2607.08846`) exact.
- Imported PDF pages rotate in the declared clockwise direction. This
  corrects an upside-down figure in `2605.23639`, reducing its page-5 raster
  difference from 388,301 pixels to six. It remains an exact-raster difference.
  The owner test checks where every cropped-page corner lands under each
  quarter-turn and unequal destination scales.

The five remaining text differences in this capture include changed paragraph
line breaks and hyphenation (`2605.20861`, `2605.29849`, `2606.12566`,
`2606.13826`, and `2607.03652`). They require layout diagnosis; extracted-text
differences are not merely PDF object-number noise.

The remaining failure, `2606.24937`, is a 588-page book. The reference
finishes in about 34 seconds; Umber reaches the unchanged 120-second limit.
A diagnostic shipout probe showed continued page production rather than an
abrupt stall, with page 125 near the cutoff. A separate shipping release
build also timed out before PDF construction, using about 1,102 MiB peak RSS.
Both probes recorded only the startup resource batch, with no later replay
batch. These observations narrow the investigation to compilation cost but
do not identify a specific hot path or justify changing the guards. The
probe code was removed.

The same repaired binary also passes all eight LaTeX/pdfLaTeX representative
consumer comparisons for TeX Live 2023–2026, recorded in
`target/arxiv-pdf-wave2-font-rotation-representatives-render/summary.json`. These small
format-loading checks retain their limited scope.

## Dated-source DVI baseline

The dated-source parity front selects the TeX Live year from each archive's
`00README.json`, and uses the same dated stable sources to build the reference
and Umber formats. The reference engine remains modern pdfTeX. See the
[detailed source-selection contract](texlive_release_selection.md#dated-source-parity-rollout-20232026).

| Declared year | Selected source snapshot | Corpus declarations                   |
| ------------- | ------------------------ | ------------------------------------- |
| 2023          | 2023-05-21               | One pdfLaTeX, one XeLaTeX             |
| 2024          | 2024-03-14               | None; representative format coverage  |
| 2025          | 2025-08-03               | 93 pdfLaTeX, two LaTeX, three XeLaTeX |
| 2026          | 2026-03-02               | None; representative format coverage  |

These are dated upstream sources. arXiv also has site configuration and
package patches, so this tier does not claim to reproduce its private
installation. Reference DVI success qualifies a paper independently of PDF
success. XeLaTeX remains explicitly unsupported by this engine. Preserve the
fixed-2026 development-kernel experiment below; its success counts cannot be
carried over to this source selection.

The completed 100-archive run in
`target/texlive-years/arxiv-modern-latex/summary.json` records 18 exact DVI
matches, covering 346 pages, 78 reference-DVI-ineligible papers, and four
unsupported XeLaTeX papers. All papers with successful reference DVI match;
there are no pending comparisons or divergences. DVI equality uses the existing
comparator's preamble-comment normalization. The ordinary resource limits
below remain unchanged.

The separate `target/texlive-years/modern-latex-representatives/` capture
contains eight exact one-page article comparisons: LaTeX and pdfLaTeX for each
of 2023, 2024, 2025, and 2026. These establish format loading and basic output
for all four releases; they do not substitute for a substantial paper corpus
for each year. The qualifying papers in this sample all declare 2025.
Both captures use `target/texlive-years/all-modern-latex-preparation.json`.
They predate the correction to standard UTF-8 format initialization above;
they are historical DVI evidence, not results for the new PDF formats.

LaTeX DVI runs retain pdfTeX primitives and engine identity, as the reference
does. Hiding them made `iftex.sty` choose different font packages in the
51-page paper `2606.27112`. Correcting that engine profile and rebuilding the
native LaTeX formats resolves the difference. Reference formats, native
pdfLaTeX formats, and runtime catalogs were reused.

Acquire the selected releases, then prepare them with the same current binaries:

```sh
for year in 2023 2024 2025 2026; do
  python3 scripts/texlive_snapshot.py acquire \
    --year "$year" --cache-root target/texlive-years
done
python3 scripts/texlive_formats.py \
  --years 2023,2024,2025,2026 \
  --snapshot-root target/texlive-years \
  --output-root target/texlive-formats \
  --reference-binary target/pdftex14029-oracle/bin/umber-pdftex14029-oracle-clean \
  --umber target/debug/umber \
  --publisher tools/texlive-wasm-publish/target/release/texlive-wasm-publish
```

The acquisition helper retains dated upstream package databases, verifies
the database’s declared release year, container lengths and SHA-512, and the
extracted inventory on reuse. An archive date alone does not establish the
release year during the annual transition. `--archive-cache PATH` reuses
package archives from another local cache only after verifying their identities.
It excludes architecture packages, documentation, and package sources. Format
preparation derives the full `language.dat` from that database and uses
stable LaTeX inputs for both engines. Each release’s own `updmap.pl`,
configuration, and authenticated Perl support generate its installed font map.
The map is available to both engines, including DVI runs that use font expansion. Each engine receives its own native
format image. Preparation also packages the complete selected TeX and font
runtime, with stable LaTeX lookup precedence and no `latex-dev` fallback.
Reference runs use a derived Kpathsea filename database over the verified
source tree. The index and its source link live in the preparation output;
source snapshots remain unchanged. This avoids recursive directory scans
without changing which source files are available.

Use `--runtime-only --preparation NEW-PATH` with an existing format output
root to refresh generated runtime configuration while reusing verified format
images. Runtime publication records belong to the new preparation receipt;
refreshing them does not rewrite the format producer receipts or earlier
corpus evidence.

Umber loads that explicit local catalog; paper runs do not depend on
recursive TeX search paths in the native file resolver. This is a local parity workflow, not the proposed public `--texlive`
package-mirror frontend.

To run the explicit DVI diagnostic comparison:

```sh
python3 scripts/run-arxiv-texlive.py \
  --source-lock scripts/pdftex-arxiv-recent-sample-100.lock.tsv \
  --archives target/parity-wave/arxiv-acquisition \
  --preparation target/texlive-formats/preparation.json \
  --umber target/debug/umber \
  --parity-harness target/parity-wave/glue-identity-bin/parity-harness \
  --output-format dvi --jobs 1 \
  --results target/texlive-years/arxiv-dvi
```

All supported rows receive reference DVI qualification before Umber parity
starts. `--qualify-only` stops after that phase; `--verify-only` checks the
recorded results without launching either engine. The ordinary 120-second,
1,536 MiB, 500-million expansion-fuel, and 10-million execution-step limits
remain the defaults. Results distinguish unsupported engines, reference
failures, pending comparisons, exact matches, Umber failures, DVI differences,
and comparator errors. A changed source tree, declaration, format, binary,
command, or output invalidates reuse rather than borrowing an earlier result.

The hermetic tooling tests run separately from expensive live parity:

```sh
scripts/check-tools.sh texlive-parity-tools
```

They test source acquisition and corruption rejection, generated format
configuration and provenance, and corpus routing/resume/result classification.
Their mocked engines do not establish TeX conformance; the live exact DVI
comparisons do. Each successful Umber run also records consumed inputs. The
runner checks common input bytes against the reference recorder and verifies
additional reads against the selected source tree or paper archive. Recorder
order distinguishes archived auxiliary inputs from later generated revisions:
a read before the first `OUTPUT` uses the authenticated archive bytes, while
reads after that write are generated inputs. A changed archive member without
a recorded write remains an error. Repeated recorder reads are hashed once
within each audit; verification starts a fresh audit. Interrupted attempts
remain available beside the resumed capture.

## Fixed-2026 development-kernel experiment

The locked recent-arXiv sample contains 100 archives. Only the 94 archives
declaring `pdflatex` enter this DVI front; the two declaring `latex` and four
declaring `xelatex` remain separately counted. The source lock and complete
archive bytes define every row. The complete source is compiled from a fresh
archive-root directory with its source-derived jobname, including archive side
files such as `.bbl` and `.aux`.

Run `scripts/survey-pdftex-arxiv-pdf.py` first with the pinned clean pdfTeX,
paired reference format, TeX Live runtime, exact archives, and a new results
directory. Its `--verify-only` pass must reconcile all 94 declared-pdfLaTeX
rows with zero compiler launches. The survey uses the `pdflatex-dev` program
profile of the paired format. The pinned `texmf.cnf` owns package, map, and
scalable-font lookup; generated TEXMF roots are isolated per row, and ambient
TeX path overrides are cleared. A PDF is successful only with a positive page
count and a completion byte count equal to its actual size, including when TeX
wraps that completion line.

After the verified PDF survey and authenticated local Umber distribution are
ready, run the serial DVI cohort. Every path below is explicit so a result
directory cannot silently change authority on resume:

```sh
python3 scripts/run-arxiv-dvi-cohort.py \
  --source-lock scripts/pdftex-arxiv-recent-sample-100.lock.tsv \
  --archives target/parity-wave/arxiv-acquisition \
  --pdf-survey target/parity-wave/arxiv-reference-pdf-paired-v3 \
  --oracle target/pdftex14029-oracle/bin/umber-pdftex14029-oracle-clean \
  --oracle-build-record target/pdftex14029-oracle/build-record.txt \
  --reference-format target/pdftex14029-reference-format/pdflatex.fmt \
  --format-receipt target/pdftex14029-reference-format/pdflatex-format.json \
  --runtime-root third_party/texlive-20260301-texmf/texmf-dist \
  --runtime-lock tests/texlive-snapshot.lock \
  --umber target/debug/umber \
  --umber-format PATH_TO_AUTHENTICATED_UMBER_PDFLATEX_FORMAT \
  --distribution PATH_TO_AUTHENTICATED_LOCAL_DISTRIBUTION \
  --distribution-ahash64 AUTHENTICATED_ROOT_AHASH64 \
  --parity-harness target/debug/parity-harness \
  --results target/parity-wave/arxiv-dvi
```

The runner verifies the PDF survey, root manifest aHash64, and the exact
`formats.pdflatex` object before any row. Each PDF-success row receives one
clean-reference DVI run. A reference DVI failure makes the row DVI-ineligible
and never launches Umber. Each DVI-eligible row receives one guarded Umber DVI
run with 500,000,000 expansion fuel, 10,000,000 execution steps, a 120-second
wall-time default, 1,536 MiB RSS default, and two-second process termination
grace. The parity harness compares existing DVI bytes with only its preamble
comment normalization. A diagnosed DVI mismatch or Umber failure stops the
cohort at that row; an internal comparator failure has a separate `ERROR`
verdict. A clean full pass is `COMPLETE`, and an unfinished prefix is `PARTIAL`.
No Umber PDF is produced or inspected in this pass.

The same invocation with `--verify-only` verifies the recorded prefix without
launching either engine. It rechecks archive, survey, authority, format,
command, jobname, outcome, and generated-artifact identities. A missing receipt
before a later row fails instead of being treated as a fresh row. The DVI pass
must finish across the eligible corpus before the separate PDF parity pass.
