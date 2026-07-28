# `umber2-johp.183`: Token-Exact Numeric Scanning and Vacuous Dimension Units

TeX82 §§442, 444, and 445 recognize numeric syntax by token identity rather
than character alone. Decimal digits and the apostrophe, double-quote, and
backtick introducers require category 12. Hexadecimal digits `A` through `F`
are the explicit exception: §445 defines both category-11 and category-12
token ranges. Lowercase letters are not hexadecimal digits. The integer and
decimal-fraction scanners now share those category-aware boundaries, including
termination and replay of a character that looks numeric but has another
category.

TeX82 §448 does not leave `scan_dimen` when §444's `scan_int` is vacuous.
After §446 reports `Missing number, treated as zero`, the scanner continues
through §§455 and 458's unit recognition and §443's optional-space scan. A
legal unit is consumed, while an illegal unit additionally reaches §459 after
the missing-number diagnostic; the completed dimension remains zero and
retains the inserted-zero recovery marker.

The table-driven Rust tests cover decimal, octal, hexadecimal, and alphabetic
introducer categories, both allowed hexadecimal categories, lowercase
terminators, recategorized integer and fraction digits, legal and illegal
vacuous units, replay, and diagnostic order. Two hermetic command-semantic
microfixtures commit the representative scanner and terminal projections. No
live reference executable runs in the correctness tier.

## Validation

After materializing the declared gitignored conformance inputs, DVI oracles,
and plain-TeX font metrics from the primary checkout, the native correctness
suite reported:

```text
run-native-tests: VERDICT: PASS - 33 packages, 48/48 test binaries, 3961 passed, 0 failed, 941 ignored; TeX82 property catalogue: 938 reviewed, 442 deferred; 100 covered, 51 gap; deferred tiers: 0 of 6 passed on this tree
```

`scripts/check.sh` reported `all 4 gates passed`.

The exhaustive differential-tracer invocation compared the committed
command-transition and geometry fixtures with zero divergences. Its overall
verdict was `PARTIAL`, not convergence, because the generated Plain, Story,
and Gentle document traces were absent from this worktree.
