# umber2-vgjr.28 -- bounded linebreak winner lookup

## Design

`record_best_route` formerly searched every winner admitted at the current
breakpoint to find the champion for one line-number and fitness class. The
active list is visited in line order, so those winners are appended in
nondecreasing line-number class order. Each class has at most the four TeX
fitness champions. Searching the final class backward therefore preserves the
existing winner slot, equal-demerit replacement, and active-list ordering while
bounding each lookup independently of the number of earlier line classes.

The implementation adds no index or other allocation. A focused regression
builds 4,096 ordered winner classes, replaces the final class champion, and
adds a second fitness champion without disturbing earlier classes.

## Measurements

Both release binaries were built uncapped with `CARGO_BUILD_JOBS=6`. Runtime
used `MemoryMax=512M` and a finite timeout. The benchmark source and budgets
were unchanged.

| Revision    | Wall time |    Peak RSS | Linebreak allocations | Requested bytes |
| ----------- | --------: | ----------: | --------------------: | --------------: |
| `27cb5476c` |  14:03.73 | 273,600 KiB |                    12 |     384,619,648 |
| `69e984f7b` |   23.23 s | 275,136 KiB |                    12 |     384,619,648 |

The identical 4,096-node workload is about 36 times faster. Allocation count
and requested bytes are byte-for-byte unchanged.

## Verification

The exact implementation commit tested was `69e984f7b6920feaa0662ee7b886872e1bd8066e`.
Commands were:

```bash
CARGO_BUILD_JOBS=6 cargo test -q --tests -p tex-typeset --no-run
CARGO_BUILD_JOBS=6 cargo build --release --manifest-path benchmarks/tex-typeset/Cargo.toml --bin layout_allocations
systemd-run --user --scope --quiet -p MemoryMax=512M /usr/bin/time -v timeout --signal=TERM --kill-after=10s 1200s benchmarks/tex-typeset/target/release/layout_allocations
systemd-run --user --scope --quiet -p MemoryMax=512M timeout --signal=TERM --kill-after=10s 180s target/debug/deps/tex_typeset-36a4e30fa8f24600 --quiet
CARGO_BUILD_JOBS=6 cargo test -q --tests --no-run
systemd-run --user --scope --quiet -p MemoryMax=1G timeout --signal=TERM --kill-after=10s 1200s env CARGO_BUILD_JOBS=6 cargo test -q --tests
CARGO_BUILD_JOBS=6 scripts/check.sh
```

All 173 `tex-typeset` tests passed under 512 MiB, covering the retained exact
semantic/physical tape, geometry, observation, overflow, and differential
proofs. The full native suite passed under 1 GiB, including the active Story
byte-exact DVI conformance gate. Uncapped `scripts/check.sh` reported all four
gates passed.
