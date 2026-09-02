# `umber2-66p0.8.40.136`: move the pending word once

## Ownership change

The `.135` authority identified the pending-horizontal-run rollback chain as
the next independent copy owner. A successful flush previously cloned the
complete `Option<PendingHRun>` before TFM processing, cloned it again into the
typed rollback lane, and then moved the original value out of `ModeList`.

TFM and OpenType word building now borrow the live source run through every
fallible step. Failure leaves that owner in place. Success moves the sole run
owner into the rollback journal and appends the completed nodes afterward, so
rollback retains a move-only receipt rather than cloned `Vec` state. Append
rollback retains a scalar source-length and identity projection. The redundant
first/current glyph copies were removed from the pending owner because the
source vector already owns those exact characters. This reduces the native
pending projection from 152 to 40 bytes and its owned value from 168 to 56
bytes.

The ordering and failure boundaries are unchanged: TFM ligature and kern
decisions, literal-hyphen discretionaries after following auto kerns, source
origins, language, semantic identity, right-boundary suppression, fuel failure,
and reverse journal replay all use the same canonical paths.

## Exact 4,096-hyphen census

The before artifact is `.135`'s integrated direct-literal-hyphen binary. The
final run used the same source, schema-12 format, authenticated offline
distribution, source-date epoch, fuel limits, copy interposer, and
`cycles:u,instructions:u` boundary. Both PDFs have SHA-256
`c0de63974d02cdb4c41cf44bff488489485268a33188e906ec5cdb1dfc037703`, and
stdout is byte-identical.

| Counter                                |                   Before |                    Final |                    Delta |
| -------------------------------------- | -----------------------: | -----------------------: | -----------------------: |
| Whole-process `memcpy` calls / bytes   |  6,173,558 / 538,527,206 |  6,146,393 / 534,025,613 |     -27,165 / -4,501,593 |
| Whole-process `memmove` calls / bytes  |     442,270 / 98,270,858 |     436,912 / 98,136,222 |        -5,358 / -134,636 |
| Hot-core allocations / requested bytes | 226,717 / 17,256,543,409 | 218,522 / 17,256,346,081 |        -8,195 / -197,328 |
| User instructions                      |            7,727,731,293 |            7,719,132,105 |     -8,599,188 (-0.111%) |
| User cycles                            |            5,957,211,755 |            3,860,325,728 | -2,096,886,027 (-35.20%) |
| User / system CPU seconds              |              2.57 / 0.21 |              1.78 / 0.15 |            -0.79 / -0.06 |
| Peak RSS                               |              144,632 KiB |              145,200 KiB |                 +568 KiB |

The former named pending take, `Option` clone, and 168-byte journal-push rows
accounted for 22,992 calls and 3,757,528 bytes in the before report and are
absent from the final top-140 table. The exact instruction, copy, and allocation
reductions are the primary results. Cycles and CPU time support the same
direction but remain host-noisy; the 568 KiB peak-RSS increase does not support
a residency claim.

The final profiling binary SHA-256 is
`84866f5323feb6c67031d90461b69429d59298758bf9488e02e1e5d45ba33b65`.
Before/final symbolized copy-report SHA-256 values are
`e218a87077197818961160bb3bf0e4d108ad3fe74d5c8f28c551958b096e556c` and
`50dee767f83a1ff3616f1ce0e0f021c7c599495cd23602b9ff1a6e28078e3205`.
Ignored evidence is under `target/umber2-66p0.8.40.136/`; the exact before
artifacts remain under `target/umber2-66p0.8.40.135/final-4096b/`.

## Validation and remaining owners

The mode-journal tests and complete `tex-exec` suite passed (759 + 4 + 24,
with the two declared executor tests ignored). The exact PDF and stdout
comparison passed. After the active logical-node lanes, the final focused
report still shows `clear_discretionary_replacements`, `evaluate_ifx`, and
third-party decompression as independent `memcpy` owners. The excluded page
arena remains the leading application `memmove` family.
