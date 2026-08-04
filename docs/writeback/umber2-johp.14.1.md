# umber2-johp.14.1 — extension registry ownership

`tex-command` now owns the TeX82, e-TeX 2.6, and pdfTeX 1.40.29 expandable
primitive identity tables and their fresh-INITEX versus format-restore
installation policy. Canonical TeX, e-TeX, and pdfTeX session/profile setup
calls that owner directly. The surviving named `tex-expand` APIs are
compatibility forwards for the retired expansion engine and its mapped
consumers; they contain no duplicate registry truth and no expansion algorithm
moved.

Focused registry coverage proves TeX82 does not acquire e-TeX or pdfTeX names,
e-TeX does not acquire pdfTeX names, each positive profile installs its exact
meaning, and format restoration reconstructs original-primitive identity
without replacing a live restored meaning. This follows e-TeX `etex.ch` §3211
and the pdfTeX 1.40.29 registration blocks catalogued in
[pdfTeX primitives](../pdftex_primitives.md).

Validation after rebasing onto `e9986985`:

- `cargo test -q --tests -p tex-command -p tex-expand`: 454 unit tests and 17
  integration tests passed for `tex-command`; 320 unit tests and 2 integration
  tests passed for `tex-expand`.
- The five-fixture differential tracer reported `VERDICT: CLEAN`, with zero
  divergences in command transitions, geometry, Plain, Story, and Gentle.
- The manual TRIP/e-TRIP fronts remained exactly at TRIP event 3919, e-TRIP
  INITEX event 2697, and e-TRIP format-loaded event 94.
- `e2e_conformance_gentle_canonical` passed its byte-exact DVI comparison.
- `scripts/run-native-tests.py`: `VERDICT: PASS - 33 packages, 48/48 test
  binaries, 4118 passed, 0 failed, 941 ignored; TeX82 property catalogue: 946
  reviewed, 434 deferred; 105 covered, 46 gap; deferred tiers: 0 of 6 passed on
  this tree`.
- `scripts/check.sh`: `check.sh: all 4 gates passed.`
