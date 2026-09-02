# `umber2-66p0.8.40.139`: direct macro-definition completion

## Attribution and boundary

TeX82 §§476--482 make parameter validation, delimiter collection, replacement
collection, and `macro_def` parameter-token rewriting one ordered scan. Umber
already selected the local-group or revision-global definition region before
that scan and appended packed token words directly. The remaining ordinary
definition boundary was metadata: the collector mirrored its parameter versus
replacement phase inside the destination, sealed a header with an unknown
origin, then mutated the published header to install the definition's primary
provenance. That second header representation required checkpoint inverse-field
edits even though the definition itself was new and unpublished until sealing.

A focused 500,000-definition release fixture used one resident source and one
open attempt, with 64 warmups and explicit reusable capacity for definition
words, headers, and source provenance. A 999 Hz `cycles:P` call-graph capture
separates broad ancestry from self work:

| Symbol                          | Baseline inclusive / self | Final inclusive / self |
| ------------------------------- | ------------------------: | ---------------------: |
| `scan_macro_definition`         |            69.40% / 0.37% |         70.41% / 0.27% |
| `scan_toks_buffers`             |            71.64% / 4.39% |         68.80% / 4.31% |
| `advance_resident_command_into` |           62.08% / 13.50% |        60.08% / 13.01% |
| `source_range_origin`           |             1.35% / 0.15% |          1.58% / 0.31% |

Thus most cycles charged inclusively to `scan_toks_buffers` are canonical
resident command delivery, not collector self work. The same capture attributes
the sampled inlined `memcpy` beneath resident delivery rather than definition
publication.

The exact public-copy interposer reconciled every process-wide caller bin. The
final run reported 135 `memcpy` calls / 33,341,595 bytes versus 136 /
33,356,634 on the exact baseline, and both reported four zero-byte `memmove`
calls. The complete symbolized reports contain no `scan_toks`, definition-arena,
seal, or origin-update copy caller: all nonzero public copies occur in source
setup, source-line firmness, generation construction, and other cold setup.
The hot definition loop itself therefore publishes no `memcpy` or `memmove`.

## Storage change

The collector phase is now the sole authority for parameter versus replacement
writes. The definition destination contains only the opaque transactional build
key. At completion, final provenance is placed in that unpublished build and
the arena writes it in the header's only publication. Sealing returns one
opaque `DefinitionRef`; there is no post-publication origin mutation.

Published definition headers are consequently immutable again. The
existing-row origin-edit representation, coalescing traversal, rollback swaps,
and acceptance replay were deleted from the definition-region mutation journal.
Failed scans and operation rollback still discard the unpublished build and
truncate its region suffix. Local-group retirement, global publication,
local-to-global `let` promotion, suspended expanded scans, and attempt-staged
`read_toks` retain their established owners; no new allocation domain or
lifetime system was added.

## Focused exact gate

Both exact binaries scanned 500,000 definitions containing 17 semantic words.
After warm capacity, both performed 8,500,000 direct word writes, 500,000 header
writes, zero second token traversals, and zero measured allocations or requested
bytes. The baseline additionally performed 500,000 post-publication header
origin writes; the final run performed zero.

Five repeated hardware-counter runs produced these arithmetic means:

| Counter                                |       Baseline |          Final |                           Delta |
| -------------------------------------- | -------------: | -------------: | ------------------------------: |
| User instructions                      | 14,536,633,437 | 14,483,120,697 |            -53,512,740 (-0.37%) |
| User cycles                            |  5,869,599,002 |  5,895,556,956 |            +25,957,954 (+0.44%) |
| Cycle run-to-run standard deviation    |          0.34% |          0.96% | no latency conclusion warranted |
| Post-publication origin writes         |        500,000 |              0 |                        -500,000 |
| Second token traversals                |              0 |              0 |                               0 |
| Measured allocations / requested bytes |          0 / 0 |          0 / 0 |                           0 / 0 |

Ignored evidence is under
`target/umber2-66p0.8.40.139-profiles/`. Baseline/final binaries have SHA-256
`b2de310dfd855c07b10db0693f9223b63e5390d89c80a5842d08fd161841c207` and
`970e878b3975036d0428d8a9b181a8b9c1d5977c3181b661fad66b4052204ba3`.
Baseline/final call-graph captures have SHA-256
`0e0cef122e95d68b387b39878409c93fa468b451fae8417f4d6893f45273af9c` and
`41c5550955e27cb665aa60ead2131ae00cf20faaa6ef837d6ac150d9af877f11`;
their symbolized copy reports have SHA-256
`e5d7d69715a371d85c5129bd3c3bb29f2d90a2de4518e8ecaa2e1f2ee2cdd219` and
`295e8c2fe85ef71fb99b8ccbd1fd67048f6894d90622bfbdf5a9a5804b992e82`.

Focused definition, `scan_toks`, and executor-definition filters pass 50, 15,
and 14 tests respectively. They cover direct local and global storage, nested
group retirement, redefinition, parameter and delimiter scanning, expanded
suspension/input retry, malformed recovery, build abort, checkpoint rejection,
and exact rollback. The complete related package run passes 387 + 23
`tex-command`, 760 + 4 + 24 `tex-exec` tests with its two declared ignored
cases, and 563 + 12 + 1 `tex-state` tests. `scripts/check.sh` reports all four
gates passed, including both clippy resolutions across 32 workspace members.
