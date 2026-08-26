# `umber2-7asg`: compact line-breaking routes

## Evidence boundary

The CPU authority is the exact authenticated ffbdb9861 arXiv 2606.12566 row
from `umber2-3gtd`: command-work vector
`(20000000,19913119,2218327,6020965,16785710,4011)`, 1,826 zero-loss
`cycles:u` samples, approximately 21,624,823,517 weighted cycles, and
1,691,000,000 cycles (7.82%) self-attributed to libc `memmove`.

An issue-local `LD_PRELOAD` interceptor then counted every out-of-line
`memmove` call and its byte size while running the unchanged base binary on the
same private distribution, format, source, environment, and 20M fuel boundary.
That census observed 67,973 calls and 7,902,964 bytes. Its warmed local command
vector was `(20000000,19913119,2218324,6020966,16785709,4011)`; the candidate
reproduced that vector exactly. The three-command difference from the retained
authority therefore predates the change and is not treated as semantic
evidence.

Material out-of-line owners, where material means at least 1% of calls or
bytes, account for more than 92% of both totals:

- format logical-row validation's temporary `BTreeMap`s: 29,639 calls and
  2,521,496 bytes;
- retained distribution selection's ordered `Vec` insertion: 239 calls and
  2,011,152 bytes;
- control-sequence string-table `BTreeMap`s under `make_string`: 16,279 calls
  and 1,410,032 bytes;
- line-breaking active-route retain and merge: 5,366 calls and 858,560 bytes;
- distribution-manifest parse-map removal and insertion: 11,171 calls and
  283,552 bytes;
- font-store map insertion: 171 calls and 205,552 bytes.

The CPU profile additionally shows many small hot-path copies which do not
dominate the byte census. Rounded direct ancestry below `memmove` assigns about
184M weighted cycles to string/tree clones, 177M to incremental-input
registration, 171M to `scan_toks_inner`, 97M each to capability refresh and
operation preparation, 84M each to framing-event drain and direct execution,
61M each to ready-operation application and conditional text skipping, 58M to
integer reallocation, and 50M each to expansion and `get_x_token`. These are
distinct owners; moving their copies into another container would not address
the underlying representations.

## Structural change

Every active line-breaking candidate formerly copied both `width_position` and
the 48-byte `Widths start_width` from a `Breakpoint`, even though the immutable
paragraph tape owns those values for the complete pass. A candidate now holds
one stable `u32` tape index and reads the successor metrics through that index.
The initial route uses a reserved sentinel. Candidate identity, ordering,
demerits, passive routes, tape lifetime, and the line-breaking passes are
unchanged; the design adds no allocation, owner, relocation, compaction, or
generation state.

`Candidate` is 80 bytes instead of 144, a 64-byte or 44.4% reduction. On the
exact 20M interceptor replay, calls fell from 67,973 to 62,680 (-5,293,
-7.79%) and bytes from 7,902,964 to 7,032,884 (-870,080, -11.01%). The two hot
retain/merge sites disappeared as out-of-line `memmove`; the remaining
200-call merge site moved 16,000 rather than 35,200 bytes (-54.5%).

A candidate `perf` capture collected 1,956 samples with zero lost and about
23,569,099,966 weighted cycles. `memmove` self was 6.20%, approximately
1,461,284,198 cycles: 229,715,802 fewer than the authority (-13.6%), and 1.62
percentage points lower (-20.7% relative share). The candidate and local base
control walls were 10.56 and 10.18 seconds respectively, so no whole-run wall
improvement is claimed from a single noisy pair. The standalone
`benchmarks/tex-typeset` allocation executable could not be rebuilt because it
has drifted from the current state APIs; that maintenance is separate from the
validated workspace tests.

Issue-local ignored evidence is under `target/umber2-7asg-memmove/`, including
base and candidate exact censuses, control runs, the candidate `perf.data`, and
build logs.

Distinct follow-up work is recorded as `umber2-5ane` for format validation,
`umber2-3z9s` for distribution selection and parsing, `umber2-6a80` for the
remaining command/execution copies, and `umber2-bcva` for the drifted standalone
typesetting benchmark.
