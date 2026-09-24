# Recent arXiv DVI Cohort

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
