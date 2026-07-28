# umber2-johp.156 — frozen endwrite identity

TeX82 §222 places `end_write` in the frozen control-sequence region, §1369
assigns that inaccessible slot the printable text `endwrite` and an
`outer_call` meaning, and §1370 injects its token directly while expanding a
write. Source lookup therefore cannot reach the stopper even though internal
write scanning can print and resolve it.

The unexpandable table now registers `endwrite` only in the immutable
primitive registry. It no longer interns or installs a mutable source control
sequence. The committed `frozen_endwrite.tex` microfixture and direct tests
cover registry access, frozen meaning and spelling, ordinary source
tokenization and `\meaning`, write termination and following-input replay, and
the distinct frozen identities used by `\cr`, `\fi`, `\par`, and alignment
templates.

Verification:

- `cargo test -q --tests -p tex-exec`: 906 passed, 0 failed.
- `cargo test -q --tests -p test-support tex82_catalogue`: 3 passed, 0
  failed.
- Command microfixtures: `tex82/command-transitions-v1` and
  `tex82/geometry-v2` each had 0 divergences. The aggregate command-stream
  verdict was `PARTIAL` because the ungenerated `plain`, `story`, and `gentle`
  document traces were deliberately not materialized.
- `scripts/run-native-tests.py`: `VERDICT: FAIL - 33 packages, 44/48 test
  binaries, 3886 passed, 5 failed, 941 ignored; TeX82 property catalogue: 938
  reviewed, 442 deferred; 100 covered, 51 gap; deferred tiers: 0 of 6 passed
  on this tree`. All five initial failures were absent local conformance
  inputs and gitignored DVI oracles for `etrip`, `gentle`, `story`,
  `story_canonical`, and `trip`.
- After copying only those ignored inputs and oracles plus the 47
  `CORPUS_TFMS` declared by the parity harness from the primary checkout, the
  authoritative rerun reported `VERDICT: PASS - 33 packages, 48/48 test
  binaries, 3967 passed, 0 failed, 941 ignored; TeX82 property catalogue: 938
  reviewed, 442 deferred; 100 covered, 51 gap; deferred tiers: 0 of 6 passed
  on this tree`. The copied conformance assets remain gitignored and
  uncommitted.
- `scripts/check.sh`: `check.sh: all 4 gates passed.`
