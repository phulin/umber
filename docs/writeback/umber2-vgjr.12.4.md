# umber2-vgjr.12.4 — migrate fixture consumers

`test-support::closed_case::FixtureCase` is now the host-facing typed boundary
for manifest-backed and Git-inventoried cases. Corpus discovery, DVI setup,
PDF parity, native bibliography invocation, classic BibTeX, distribution, and
command-semantic regeneration use it for safe payload access and exact staged
membership. Classic BibTeX imports its declared roles, compatibility profile,
and hashes into the shared contract instead of repeating those validators in
its tests.

The shared contract now preserves manifested versus unmanifested candidate
shape and owns the sole canonical `case.inventory` serializer. Fixturegen uses
that serializer for classic and PDF layout candidates. Single-case PDF and
ordinary text regeneration stage typed candidates and publish through
fixturegen's existing atomic transaction; whole-area PDF and command-semantic
regeneration use the same plan/apply owner. No test helper or command tool can
publish fixture authority. No committed corpus evidence changed.

Focused consumers and command tooling passed after uncapped `--no-run` builds
and under a 512 MiB cgroup. All 33 fixturegen tests passed under the same cap,
including negative inventory and atomic rollback/dry-run coverage. The full
workspace passed after an uncapped `--no-run` build and under a 1 GiB cgroup.
`scripts/check.sh` passed all four gates under 1 GiB with one Cargo build job;
two parallel cold clippy attempts exceeded that cap before the serialized run
completed both declared lint resolutions.

Before this writeback, the issue diff deleted 173 authored lines and no
declarative fixture lines. It added 540 authored lines and 64 generated lockfile
lines, for authored net +367 and generated/declarative net +64. The added
contract adapters and adversarial staging proof are retained infrastructure;
the deletions are the repeated consumer validation, inventory serialization,
and direct mutation paths. The earlier command-semantic and TeX82 child issues
reported their own declarative deletions separately.
