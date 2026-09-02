# `umber2-66p0.8.40.143`: borrowed valid input through line firming

## Adopted boundary

TeX82 §363 displays the normalized file line and asks for terminal input only
when `\pausing>0` and interaction is above nonstop mode. A bare return leaves
the line in place; a nonempty reply replaces it. Umber already represented the
line as a range into the source slot's immutable registered backing and already
gave a real replacement its own `line_backing` owner, but it converted every
valid range to an owned `String` before lending `&str` to
`SourceStepQueries::firm_up_the_line`.

The hook now receives the direct `String::from_utf8_lossy` projection. Its
ordinary valid branch is a borrowed `str` over the resident backing. Unicode
sources remain completely UTF-8-validated at registration; exact-byte sources
retain their existing arbitrary-byte semantics and materialize a transient
lossy display string only when the selected line is not UTF-8. `None` adds no
owner. `Some(SourceRegistration)` still enters the existing checked
registration and source-history transition, so replacement provenance,
source framing, line number, buffer accounting, rollback, and retirement are
unchanged. No persistent `Cow`, cache, alternate line representation, or input
stack state was added.

## Exact focused evidence

The optimized profiling test executable runs the identical one-line and
4,096-line physical acquisition/firm/finish loops against exact-byte valid
input. Each hook records whether its `str` points into the registered backing,
and the profiling allocator records calls and requested bytes in the enclosing
delivery-and-scan scope.

| 4,096-line result                         |    Exact base | Candidate |
| ----------------------------------------- | ------------: | --------: |
| Hook calls                                |         4,096 |     4,096 |
| Direct borrows of registered backing      |             0 |     4,096 |
| Warmed allocation calls / requested bytes | 4,096 / 4,096 |     0 / 0 |

The one-line row changes identically from zero direct borrows and one
allocation/one requested byte to one direct borrow and zero allocations. Thus
the valid-line owner has an exact one-for-one removal: the `.140` ordinary
56,964-copy / 1,842,726-byte row has zero valid-line remainder. The only
irreducible display allocation is an invalid-UTF-8 exact-byte line; a genuine
nonempty terminal reply separately retains the one replacement backing that
TeX semantics require.

Seven alternating `perf stat` pairs ran those same exact-base and candidate
executables over `cycles:u,instructions:u` without the allocation scope being
changed. Instructions fell in all seven pairs and cycles fell in all seven;
the means were:

| Counter           | Exact base | Candidate |                Delta |
| ----------------- | ---------: | --------: | -------------------: |
| User instructions |  3,264,093 | 2,087,335 | -1,176,758 (-36.05%) |
| User cycles       |  3,686,223 | 2,505,468 | -1,180,755 (-32.03%) |

This intentionally small microfixture makes no end-to-end latency claim. The
instruction count is stable across pairs (paired deltas from -36.01% to
-36.12%); cycles are supporting context for the exact owner/allocation proof.

Exact-base and candidate binaries have SHA-256
`33524160c3efb512aad7f17a41aba133e83a20f93dc9d033f9dcbf630615b81b`
and
`494177198013ba0b2e3a191853f41d15188e7099ddb51186e27b693c0e616f8c`;
their build IDs are `51d3409eefc9055f7080456dc24249414223ece7` and
`9ceec62013571462fe03232b92b2000524c9244e`. Ignored raw evidence is under
`target/umber2-66p0.8.40.143/focused-gate/`.

## Semantic and lifecycle coverage

The focused tests prove the backing-address borrow for a non-ASCII Unicode
file line, an ordinary terminal line, and an empty `\scantokens` line. A
separate exact-byte test preserves the prior replacement-character display for
malformed UTF-8. Existing tokenizer tests exercise batch/no-pausing, an
interactive bare-return line, a nonempty interactive replacement, trailing
blank normalization, empty lines, and enabled/disabled profile-specific
`\endlinechar`. The source replacement checkpoint test now installs an actual
typed replacement and proves candidate rewind/redo restores its sole backing
owner and buffer charge.

The complete command suite additionally covers exact-byte preservation,
source identities and framing, file/read/terminal retirement, source cursor
rollback, and input suspension with exact prefix replay. The complete executor
suite covers e-TeX `\scantokens`, `\read`/`\readline`, terminal acquisition,
interactive replacement before suspended input, resource resumption, and
operation rollback.

Validation results:

- `cargo test -q --tests -p tex-command`: 391 unit and 23 boundary tests pass.
- `cargo test -q --tests -p tex-exec`: 760 unit tests pass with two ignored,
  plus four main-control and 24 external boundary tests.
- `scripts/check.sh`: all four gates pass; both Clippy resolutions are clean
  across 32 workspace members.
