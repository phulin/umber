# Recent arXiv census captures

This directory preserves machine-readable captures for the corpus selected by
`scripts/pdftex-arxiv-recent-sample-100.tsv`. The source identities and selected
entrypoints are pinned by `scripts/pdftex-arxiv-recent-sample-100.lock.tsv`.

`recent-20260723-non-pdflatex/` records the six recent-sample rows whose
reference compiler is LaTeX or XeLaTeX rather than pdfLaTeX. Its metadata
records the exact sample, source-lock, Umber binary, distribution, format, and
guard identities. The capture is intentionally partial and must not be treated
as a complete 100-row engine census.

`recent-20260902-pdftex-pdf/` records the clean TeX Live 2026 PDF-success
denominator for all 94 unique declared-pdfLaTeX rows. The clean oracle produced
87 authoritative PDFs; six rows stopped at `Undefined control sequence`, and
one timed out with only a non-authoritative partial PDF. Its compiler-free
verification receipt reproduces the ordered report and exact totals after
rehashing every retained input and artifact. This capture contains no Umber PDF
comparison.
