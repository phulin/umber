# TeX Live 2026 pdfLaTeX PDF-success denominator

This capture is the reference-only PDF-success denominator for the locked
recent-arXiv sample. It accounts for each of the 94 unique rows whose archive
`00README.json` declares `pdflatex`, exactly once. The other two `latex` rows
and four `xelatex` rows are outside this report.

The clean pdfTeX 1.40.29 oracle produced an authoritative PDF for 87 rows. Six
rows exited at their first `Undefined control sequence` error, and one row hit
the 120-second guard after creating a non-authoritative partial PDF. The exact
failure line, bounded following context, terminal status, and any partial PDF
identity remain in `results.jsonl`:

- `2606.26017`, `2606.00201`, `2607.09509`, `2605.19153`, `2606.20080`, and
  `2607.09020`: `Undefined control sequence`, with no PDF.
- `2606.12566`: timeout status 124, with the partial PDF presence, byte length,
  and SHA-256 recorded but excluded from the success count.

Every run used a fresh exact archive root as its working directory, invoked the
locked entrypoint without `--jobname`, and therefore retained TeX's
source-derived `\jobname`. Archive-provided and generated side files stayed in
the issue-local row directory. No paper was patched. The runtime tree, clean
oracle, oracle build record, paired clean format, format receipt, command, and
guard identities are recorded in `metadata.json` and repeated in each row where
required for independent accounting.

The report identities are:

- `results.jsonl`: SHA-256
  `68303516775a704d954bd6484c39d0dba8869cecf28353a3147be9cffdc1b83c`.
- `summary.json`: SHA-256
  `fb64d1f63eba232dee9f5091494140c1cebd4fd799495681a92188a2fa6bba74`.
- clean oracle: SHA-256
  `608cb1760e9a471668ba97eea22fde60f2f7fadd285acd7c7b1ba243ddf71db3`.
- clean pdfLaTeX format: SHA-256
  `d557a9b8d7de1ff9bc5a570830701417d32f09b300f0188f9a18a485aaf1ff8e`.
- TeX Live runtime tree: aHash64 `1cd780f1ca7b3648`, authenticated by
  `tests/texlive-snapshot.lock` SHA-256
  `836b9133624f2deb7a59de3159a66ca08b653635b300019fb49179e6dae30621`.

`verification.json` is the receipt from
`scripts/survey-pdftex-arxiv-pdf.py --verify-only`. That mode launched zero
compilers, rehashed the locked sources, authority, source views, logs, and PDF
artifacts, reproduced the ordered JSONL and totals, and verified all 94 rows.
Neither the survey nor verification invoked Umber or inspected an Umber PDF.
