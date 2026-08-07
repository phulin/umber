# umber2-vgjr.23 -- fixture-contract forecast reconciliation

## Owner decision

The portfolio owner accepts Program 12 at its measured 503 lines of net
authored source/configuration growth and retires the original 500--900-line
authored-reduction forecast. No unimplemented deletion remains scheduled, and
no declarative fixture, generated schema, generated lockfile, moved, historical,
documentation, binary, or total-line change is credited as authored reduction.

The forecast correctly identified the detached capture list, duplicate census,
repeated V1 manifest fields, generated default-disposition catalogue, repeated
consumer validators and serializers, and direct publication paths. It
underestimated the typed replacement contract and the independent negative
proof required at the Git-authority, local-candidate, and atomic-publication
boundaries. Moving from the measured 503-line growth to the forecast floor
would require another 1,003 net authored deletions; the audit found no such
duplicate implementation.

## Authority and caller audit

- `test_support::git_fixture::ClosedCase` is the sole Git-checkout authority.
  It validates normalized repository-relative paths, real directory ancestry,
  tracked regular-file membership, declared order, and the exact on-disk tree,
  then revalidates before safe payload access.
- `test_support::closed_case::FixtureCase` is the sole typed consumer contract.
  It binds the Git case to identity, ordered file roles and optional hashes,
  exact source closure, status and xfail metadata, execution profile, and
  publication metadata. Corpus discovery, DVI/PDF consumers, bibliography,
  distribution, and command-semantic regeneration enter through it rather than
  maintaining family-local validators.
- `test_support::closed_case::StagedCase` and
  `candidate_inventory_bytes` are the sole non-authoritative candidate
  validation and canonical `case.inventory` serialization boundary. They
  reject unsafe or non-regular entries and missing, extra, duplicate, and
  reordered membership without gaining permission to publish.
- Fixturegen's `AtomicCaseTransaction` is the sole mutation and publication
  authority. Ordinary text, PDF, classic BibTeX, and command-semantic cohort
  routes hand it complete validated candidates; its transaction owns planning,
  clean Git authorities, collision rejection, rollback, retry, and
  all-or-nothing installation.
- `tex-command-stream`'s V2 manifest remains the sole command-semantic case,
  capture-policy, route, channel, status, and strict-xfail authority for all 203
  fixtures. The shell regeneration entry point selects that typed policy and
  does not retain the deleted 173-line capture catalogue or 467-line census.
- The TeX82 catalogue gate is the sole resolved-disposition authority. It
  initializes the exact 1..=1380 inventory with one typed deferred default,
  applies sorted explicit shard overrides, and pins the resolved digest and
  946 reviewed / 434 deferred module plus 106 covered / 45 gap property census.
- `corpus-manifest` remains a dependency-free parsing leaf. Its acquisition
  caller does not import test-support or fixturegen and cannot publish fixture
  authority.

The superficially similar checks defend distinct trust transitions: committed
Git authority, an untrusted local candidate, and atomic replacement. Collapsing
them would merge authority with mutation or permit ambient bytes to become
evidence. The retained tests are also independent behavior owners for exact
membership and order, explicit expected values, hashes, source closure, xfails,
capture selection, census, traversal, local edits, rollback, retry, and partial
publication. They are not shadow implementations available for line-count
deletion.

## Exact accounting

Independent `git show --numstat` classification of implementation commits
`5675a359f`, `fbcaa9616`, `0634d2800`, `5a012b9e7`, and `1aceb8ae8` reproduces
the Program 12 totals.

| Category                  | Additions | Deletions |     Net |
| ------------------------- | --------: | --------: | ------: |
| Authored source/config    |     1,676 |     1,173 |    +503 |
| Documentation             |       136 |        33 |    +103 |
| Declarative fixtures      |     6,834 |    21,244 | -14,410 |
| Generated schema evidence |       443 |       199 |    +244 |
| Generated lockfiles       |       333 |         8 |    +325 |
| Complete tracked change   |     9,422 |    22,657 | -13,235 |

No binary fixture changed. The 14,410-line declarative reduction exceeds its
own forecast, but it and the generated categories remain separate from the
authored result. The owner decision retires the forecast rather than converting
those categories or gross predecessor deletion into unsupported credit.

## Verification

With `CARGO_BUILD_JOBS=6`, uncapped focused consumer and fixturegen builds
passed. Focused contract, inventory, validation, staging, census, traversal,
local-edit, and atomic-publication execution passed under `MemoryMax=512M`.
The complete native suite and fixture tooling passed under `MemoryMax=1G`, and
uncapped `scripts/check.sh` passed all four gates. Every runtime used a finite
timeout; no `prlimit` was used.
