# umber2-awgc.1.1: Uninstrumented Pinned arXiv Baseline

## Immutable authority

The sole no-peer row used optimized production-default Umber at commit
`ce1d7cd8f57cb6e16ce7c7403f799abcb55ed1b4`. The release-like Cargo
`profiling` profile retained symbols but enabled no Cargo feature. The binary
is 332,121,032 bytes with SHA-256
`ab3df68860638cfc644dd77a0e1da7deb86e2daf9ebd12da718a8c323ab321df`.
No production source, fixture, distribution, cache, guard, or integrity-policy
change was made.

The reconstructed `2606.12566` authority is:

- source archive: 46,654,107-byte `2606.12566.src`, SHA-256
  `05a491fc231c85c5827f1dd1b41f80c361f300898d2b3830601c121b0e6d8a2a`;
- selected `ArXiv.tex`: SHA-256
  `816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`;
- prepared schema-11 pdfLaTeX format: 2,030,553 bytes, SHA-256
  `32ae8a46f86ecc3520b48ff6739fa413170f7b34c2263560d7d589abe1466a7b`;
- authenticated sparse schema-3 distribution root: SHA-256
  `560ab65f2a4933879b05e47554a9d94434ec1e94ff8f6caa163d26cde7fe35bd`,
  derived from base root
  `61b8d665e492662b18c8beb70ab8cd8a8f73d9bd7e4d9aeb2f958ea8613f8883`;
  and
- ordered runtime closure: 105 unique keys, SHA-256
  `75d85bb12f8fa5eba0ae2a42daf73fd86c44852ecdc230196455b9aea24565b5`.

The original physical cache was reused in place. It contained 528 regular
files, 264 non-lock blobs, and no symlink. Its normalized path/size inventory
is
`7c5f22269ce1735a234b5ee1b1d0ea05bded69d9047a06c973fbd994e0957bbb`.
The complete content listing is
`921e9a52ae72d97f6146644999dec7ddc777f952288b3fd8ab24e8fd07810e51`
under the historical `en_US.UTF-8` ordering and
`37c22368f87e4216cd0759963e0fd2faa9423977094d89f932c05e54a9540b1b`
under the measured host's `C.UTF-8` ordering. Both historical digests were
reproduced. Byte and normalized inventories compared equal before and after
the row. Offline stderr contains no `umber: acquired` record.

## Exact command and guards

The fully expanded Umber argument vector is evidence SHA-256
`9c1ad960e6cf5b3be580e2334e2b2cabf6b76198a58c107378eb0de7629ba6d7`.
It selects `run --pdflatex`, the prepared format and explicit distribution
above, `--distribution-sha256` with the schema-3 root, `--offline`,
`--expansion-fuel 100000000`, the 105 ordered `--prefetch-input` keys,
`--pdf`, `--input-records-out`, and `ArXiv.tex`. The complete guarded vector is
SHA-256
`a12907f66adbdd0c5e788b737f376b26ae8a1a1ab1316ceaaa03cf81b9d3ba84`.
It uses `scripts/run-umber-guarded.py` SHA-256
`3389d8e5167af44d255cf64bcba9908a1857e2778b9ed3bc2fb6442fc240a063`
with 120 wall seconds, 1,536 MiB aggregate RSS, and a two-second TERM grace.
Default ErrorStop interaction with terminal EOF, the 100,000,000-action
limit, unrestricted host affinity `0-23`, and unset `TEXINPUTS`/`TEXFONTS`
were unchanged. The cache root was selected only through `XDG_CACHE_HOME`.

The exact observer is SHA-256
`f0ebf392c504afa271dc87dcb8ed78c0decd59c6a9a32a39ec1d6545f161168e`.
Before-row process evidence contained no `cargo`, `rustc`, `umber`, `perf`, or
Samply peer; the issue wrapper itself was the only matching shell command.

## Result

The wall guard fired and the fully reaped process group returned status 124.
Full-process time was 120.976969 seconds wall, 138.56 seconds user, and 11.97
seconds system. The two-second external observer recorded 60 samples and a
659,172-KiB aggregate RSS peak at 118.929023 seconds; `/usr/bin/time` reported
657,992 KiB maximum RSS. There were zero major faults, zero swaps, 397,536
minor faults, 1,012 voluntary context switches, and 570 involuntary context
switches.

Production startup telemetry recorded source read 139,926 ns, format read
1,539,502 ns, format restore 95,355 ns, and complete setup 2,530,796 ns. These
are the only engine-owned milestones exposed by this production-default
binary. Its strings and current source contain no `UMBER_PROFILE_PROGRESS`
hook; setting that historical environment variable produced no progress file,
no exact committed-step/fuel endpoint, and no command-work vector. The receipt
does not impute those values from an instrumented predecessor. Integrated
issue `umber2-awgc.1.3` owns the matched exact-work and output census built on
the profiling-only instrumentation from `umber2-awgc.1.2`.

The wall guard fired before publication. Neither `document.pdf` nor
`inputs.tsv` exists, so production published no output or output identity. An
empty stdout SHA is process evidence, not a fabricated semantic-output
identity.

Host pressure totals changed by 772,558 microseconds CPU some and zero CPU
full, 2,952 microseconds memory some and 2,848 memory full, and 65,225
microseconds I/O some and 63,204 I/O full. Memory PSI `avg10` was zero before
and after. The containing cgroup was uncapped; its monotonic memory events are
all zero after the row, proving zero low, high, max, OOM, OOM-kill, and
OOM-group-kill events over the row. The unchanged 1,536-MiB aggregate guard,
not a looser cgroup limit, remained the memory authority.

## Comparison and evidence

The prior fixed-work authority at commit
`f6509d1ca81107a6f727fd02da74370c0b8026ec` has assessment SHA-256
`743cbb456e4902592bd1391891018f70c2c9ec9313257adaedd470ce05a35200`.
It used the same pins, closure, cache, guards, two-second observer, and
interaction contract, but linked the temporary `profiling` progress observer.
It reached the same wall status in 120.896614 seconds with a 558,820-KiB
sampled peak. The present row is 0.080355 seconds later at the guard and
100,352 KiB, or 17.96 percent, higher in sampled peak RSS. Wall deltas between
two guard terminations are not throughput evidence. The prior step/fuel
milestones likewise cannot be compared because the current production row
exposes no exact-work endpoint.

The separate production-feature equal-work authority at the same prior commit
ended exactly at 6,000,000 and 12,000,000 fuel in 8.866 and 16.322 seconds.
Those bounded prefixes remain CPU-attribution context only and are not
silently combined with this full-process wall/RSS row.

All issue evidence is namespaced under `target/umber2-awgc.1.1`. The final
manifest of row artifacts has SHA-256
`9e3a8790539912a1ffc0e8fd4c4063b9c220a3025eebd79bb9c705feffaa6f2b`;
primary row-result, RSS-sample, process-time, and
stderr SHA-256 values are respectively
`567bdd816e1738e3b4a3613c3cd083f6248bb4c99e532e1d6cbca4aba9ad8fdb`,
`2d7a92bcb42c891b6a9771e41e4eb3ff075af4078e74e7092cc97450c3ad474f`,
`c55fbf863d5f987ec574371e872ac988f2e097ab08e8cfaa6f3f9c70753ae54e`,
and
`72513b301820bb5780aa073d0f07bf2db2db167ecf913b1413bd578630bdffc3`.
