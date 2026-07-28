# umber2-johp.161 — macro parameter diagnostics

Authority: TeX82 `tex.web` §§476, 479, and 1218.

The macro-definition scanner now owns two distinct recoverable errors:

- §476 rejects a nonconsecutive parameter-text number, backs up the rejected
  follower, inserts the expected parameter slot, and reports
  `Parameters must be numbered consecutively` with its two-line help.
- §479 rejects an illegal replacement-body parameter number, backs up the
  rejected follower, stores the parameter character, and reports
  `Illegal parameter number in definition of` followed by the defining
  control sequence and its three-line help.

The defining target travels with the structured scan, preserving the
named-versus-active namespace distinction used by `sprint_cs`. Diagnostics are
printed at the scanner recovery boundary instead of being collapsed into a
post-scan boolean, so backup, diagnostic, inserted-token delivery, and
definition commit retain canonical order. Recovered definitions still obey
the ordinary local/global assignment policy.

Coverage includes the focused Rust matrix
`macro_parameter_errors_have_distinct_tex82_diagnostics_and_commit_scope` and
the committed `main-control/macro-parameter-errors` semantic microfixture.
Together they exercise valid doubled parameter characters, missing and
nonconsecutive numbers, named and active targets, nested replacement braces,
and local/global recovered definition commits.

Validation:

- Focused `tex-command`, `tex-exec`, 100-error-limit, property-catalogue, and
  command-semantic microfixture tests pass.
- `scripts/run-native-tests.py`: `VERDICT: FAIL - 33 packages, 44/48 test
  binaries, 3891 passed, 5 failed, 941 ignored; TeX82 property catalogue: 938
  reviewed, 442 deferred; 100 covered, 51 gap; deferred tiers: 0 of 6 passed
  on this tree`. The five failures are the required local Story, Gentle, TRIP,
  e-TRIP, and canonical Story DVI oracles, which are absent from this
  worktree; no available native test failed.
- After copying the exact existing ignored inputs, declared TFMs, and DVI
  oracles from the owning checkout without regenerating or modifying them,
  `scripts/run-native-tests.py`: `VERDICT: PASS - 33 packages, 48/48 test
  binaries, 3972 passed, 0 failed, 941 ignored; TeX82 property catalogue: 938
  reviewed, 442 deferred; 100 covered, 51 gap; deferred tiers: 0 of 6 passed
  on this tree`.
- `scripts/check.sh`: `all 4 gates passed`.
