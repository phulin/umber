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

## Validation

The first authoritative native run found an environmental prerequisite gap:

```text
run-native-tests: VERDICT: FAIL - 33 packages, 44/48 test binaries, 3872 passed, 5 failed, 941 ignored; TeX82 property catalogue: 938 reviewed, 442 deferred; 100 covered, 51 gap; deferred tiers: 0 of 6 passed on this tree
```

All five failures named absent gitignored conformance inputs or DVI oracles.
After materializing only those declared inputs, the four DVI oracles, and the
47 declared plain-TeX TFM files from the primary checkout, the authoritative
rerun completed:

```text
run-native-tests: VERDICT: PASS - 33 packages, 48/48 test binaries, 3953 passed, 0 failed, 941 ignored; TeX82 property catalogue: 938 reviewed, 442 deferred; 100 covered, 51 gap; deferred tiers: 0 of 6 passed on this tree
```

The copied prerequisites remain gitignored and are not part of this change.
`scripts/check.sh` also reported `all 4 gates passed`.
