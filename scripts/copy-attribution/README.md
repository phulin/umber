# Exact public-copy caller attribution

This profiling-only Linux probe interposes public `memcpy` and `memmove`. A
direct call from the application executable is binned by its PIE-relative
return address. An external direct caller takes a bounded `_Unwind_Backtrace`
and is binned by the nearest application return address; a stack with no
application ancestor remains an `external_only` bin identified by module and
module-relative address.

The hot table is fixed at 32,768 bins per API with at most 24 probes per
insertion. The report publishes collision probes, maximum probe distance,
overflow calls and bytes, and interposer-internal calls suppressed while
attribution itself runs. Table overflow is an explicit caller bin, so caller
calls and bytes still sum exactly to each API total. Collection starts after
the interposer constructor has resolved libc and the main executable's ELF
segments, and stops before reporting.

Build and use the probe with an exact debuginfo-bearing binary:

```bash
cc -shared -fPIC -O2 -g -fno-builtin-memcpy -fno-builtin-memmove \
  -Wall -Wextra -Werror -o target/copy_attribution_probe.so \
  scripts/copy-attribution/copy_attribution_probe.c -ldl
UMBER_COPY_ATTRIBUTION_OUT=target/copy.raw \
  LD_PRELOAD="$PWD/target/copy_attribution_probe.so" target/profiling/umber ...
scripts/copy-attribution/symbolize.py \
  --binary target/profiling/umber --report target/copy.raw >target/copy.txt
```

The symbolizer independently reconciles parsed bins with both totals, ranks
each API by bytes, records the exact binary SHA-256, and uses `addr2line -i` to
publish the source and inline chain for every ranked application bin. It fails
if a ranked application address does not resolve completely against that
binary. Run the hermetic scalar, `Vec`, and external-ancestor gate with:

```bash
scripts/check-tools.sh copy-attribution
```
