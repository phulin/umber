# Recent arXiv census captures

This directory preserves machine-readable captures for the corpus selected by
`scripts/pdftex-arxiv-recent-sample-100.tsv`. The source identities and selected
entrypoints are pinned by `scripts/pdftex-arxiv-recent-sample-100.lock.tsv`.

`recent-20260723-non-pdflatex/` records the six recent-sample rows whose
reference compiler is LaTeX or XeLaTeX rather than pdfLaTeX. Its metadata
records the exact sample, source-lock, Umber binary, distribution, format, and
guard identities. The capture is intentionally partial and must not be treated
as a complete 100-row engine census.
