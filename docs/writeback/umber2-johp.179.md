# `umber2-johp.179`: Restricted-Integer Recovery Reports

TeX82 §§433-437 now publish every rejected bounded integer through one typed
command-to-executor recovery channel. The record preserves the restricted
class and raw `scan_int` result while the scanner returns TeX's recovered zero;
the executor drains records in detection order and renders §73/§79/§91's
`print_err`/`help2`/`int_error` text through the terminal/log sink.

The channel covers math characters and accents, numeric delimiters and
radicals, `\left`/`\right` and all three `withdelims` fractions, all three math
family assignments, and the classical register operands used by count,
dimension, glue, muglue, token, and box commands. Box scans now use §433's
bounded scan directly, so `\setbox`, box reads, shifts, leaders, and `\vsplit`
also preserve the recovered zero instead of raising an executor range error.

The direct scanner matrix pins both boundaries for all five classes and the
exact ordered recovery payload. Canonical replay matrices pin diagnostic text,
the raw rejected integer, repeated-report order, recovered values, primitive
variants, and local/global family assignment semantics. The property catalogue
records those executable owners; no live reference executable runs in the
correctness tier.
