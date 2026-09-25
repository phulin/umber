# Recent arXiv DVI Cohort

## Dated-source front

The next parity front selects the TeX Live year from each archive's
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
stable LaTeX inputs for both engines. Each engine receives its own native
format image; the existing local resource provider supplies the selected
runtime. This is a local parity workflow, not the proposed public `--texlive`
package-mirror frontend.

Then qualify and compare the complete source bundles:

```sh
python3 scripts/run-arxiv-texlive.py \
  --source-lock scripts/pdftex-arxiv-recent-sample-100.lock.tsv \
  --archives target/parity-wave/arxiv-acquisition \
  --preparation target/texlive-formats/preparation.json \
  --umber target/debug/umber \
  --parity-harness target/parity-wave/glue-identity-bin/parity-harness \
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
comparisons do.

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
