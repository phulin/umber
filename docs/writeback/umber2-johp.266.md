# umber2-johp.266 — script target reservation

TeX82 §1176 selects the superscript or subscript field pointer before calling
§1151's `scan_math`. If §687 disallows the current tail or the selected field
is nonempty, §1177 first appends an ordinary dummy noad. Only the nonempty-field
case runs the exact `Double superscript` or `Double subscript` diagnostic,
`help1`, and `error`; all of those effects precede the field scan.

Canonical main control represents the selected pointer as a stable mlist node
index. It reserves that target and emits §1177 recovery before scanning, then
fills the same field after an unbraced §1151 value or a live §§1151–1153 math
group completes. Nodes appended while recovery or nested script execution runs
therefore cannot redirect the field to a newer tail.

Focused coverage challenges both script kinds, eligible and disallowed tails,
empty and occupied fields, braced and unbraced fields, exact message/help/output
ordering, nested scripts, later tail appends, and local scalar provenance. The
`tex82.math.script-attachment` semantic microfixture commits duplicate-script
execution observations and the resulting DVI artifact hash.

Validation:

- `cargo test -q --tests -p tex-command`: 436 unit tests and 17 integration
  tests passed.
- `cargo test -q --tests -p tex-exec`: 922 unit tests and 3 integration tests
  passed.
- `scripts/run-native-tests.py`: `VERDICT: PASS - 33 packages, 48/48 test
  binaries, 3994 passed, 0 failed, 942 ignored; TeX82 property catalogue: 946
  reviewed, 434 deferred; 103 covered, 48 gap; deferred tiers: 0 of 6 passed on
  this tree`.
- `scripts/check.sh`: `check.sh: all 4 gates passed.`
- The differential tracer compared the committed command-transition and
  geometry fixtures cleanly. Its overall verdict was `PARTIAL` because this
  linked worktree has no generated plain, story, or Gentle document traces;
  no document trace was generated or modified for this issue.
