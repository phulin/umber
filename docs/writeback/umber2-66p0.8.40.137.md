# `umber2-66p0.8.40.137`: keep `\ifx` operands stationary

## Distinct residual seam after `.43`

`umber2-66p0.43` is present and remains the semantic comparison design:
`evaluate_ifx` borrows each `ResolvedMeaning`, and macro equality borrows the
parameter and replacement spans through the existing `DefinitionRef` values.
No token list is cloned or materialized by that helper.

The later public-copy census identified a separate command-delivery
relocation. The scanner-restoration closure returned its two
`Option<CurrentCommand>` operands as a tuple. Each current command is 72 bytes,
so every completed predicate emitted one 144-byte `memcpy` after both raw
deliveries. The earlier broad row was 94,786 calls / 13,649,184 bytes, exactly
144 bytes per call.

The closure now compares the two stationary caller-local command slots through
borrowed meanings and returns only the boolean across scanner restoration.
Raw delivery, outer-control legality, scanner restoration on every result,
suspension behavior, provenance, and the `.43` definition-span comparison are
unchanged. There is no cache, scan, allocation, semantic shortcut, or new
owner. The architecture contract did not change, so no architecture document
needed revision.

The boundary test requires the borrowed comparison inside the closure and
rejects returning `(first, second)`. The semantic matrix covers identical and
distinct primitives, different undefined control-sequence spellings, an active
character and escaped control sequence with the same raw meaning, and character
code/category equality. The existing macro matrix continues to cover distinct
definition identities with equal contents, unequal flags, unequal parameter
text, unequal replacement text, and macro-versus-undefined comparison.

## Exact focused comparison

The persisted `.136` result is the before row. The final run used the same
4,096-hyphen pdfLaTeX source, schema-12 format, explicit offline distribution,
source-date epoch, fuel limits, output path shape, checked public-copy
interposer, and `cycles:u,instructions:u` boundary. That fixture executes 3,676
`\ifx` predicates while retaining the neighboring focused TFM workload. The
source SHA-256 is
`3eb9f9915f8640525a4acc95b621f833de4b7e31268b41f5a3feb2fab28621e8`.

| Counter                                |                   Before |                    Final |                 Delta |
| -------------------------------------- | -----------------------: | -----------------------: | --------------------: |
| `evaluate_ifx` `memcpy` calls / bytes  |          3,676 / 529,344 |                    0 / 0 |     -3,676 / -529,344 |
| Whole-process `memcpy` calls / bytes   |  6,146,393 / 534,025,613 |  6,142,717 / 533,496,269 |     -3,676 / -529,344 |
| Whole-process `memmove` calls / bytes  |     436,912 / 98,136,222 |     436,912 / 98,136,222 |                 0 / 0 |
| Hot-core allocations / requested bytes | 218,522 / 17,256,346,081 | 218,522 / 17,256,346,081 |                 0 / 0 |
| User instructions                      |            7,719,132,105 |            7,718,553,413 |  -578,692 (-0.00750%) |
| User cycles                            |            3,860,325,728 |            3,895,012,431 | +34,686,703 (+0.899%) |
| User / system CPU seconds              |              1.78 / 0.15 |              1.62 / 0.12 |         -0.16 / -0.03 |
| Peak RSS                               |              145,200 KiB |              145,388 KiB |              +188 KiB |

The exact whole-process reduction equals the deleted row, and `evaluate_ifx`
is absent from the final symbolized report. `memmove`, the complete hot-core
census, and every hot-core allocation category are byte-for-byte unchanged.
The 14,656-byte PDFs are identical with SHA-256
`c0de63974d02cdb4c41cf44bff488489485268a33188e906ec5cdb1dfc037703`;
stdout is also byte-identical. The instruction reduction is the primary CPU
result. Cycles, CPU time, and the 188 KiB peak-RSS difference are host-noisy
supporting observations and do not support a residency claim.

Baseline/final profiling binary SHA-256 values are
`84866f5323feb6c67031d90461b69429d59298758bf9488e02e1e5d45ba33b65`
and `e65cede912f916b9da2c2d4a4d544857e728b273076eacf805ec5820335c244d`.
Baseline/final symbolized copy-report SHA-256 values are
`50dee767f83a1ff3616f1ce0e0f021c7c599495cd23602b9ff1a6e28078e3205`
and `d750da58ae27c62d53d2f6a8e252759ded7a8a5a6356bb3d36a5c74ea1a867b`.
Ignored final evidence is under `target/umber2-66p0.8.40.137/`; the exact before
artifacts remain under `target/umber2-66p0.8.40.136/final-4096/`.

## Validation

- `cargo test -q -p tex-command --tests ifx`: 3 passed.
- `cargo test -q -p tex-command --test it condition_delivery_and_alignment_lifecycle_remain_on_the_canonical_seams`: 1 passed.
- `cargo test -q -p tex-command --tests`: 386 unit and 23 integration tests passed.
- `scripts/check.sh`: all repository gates passed.

Linked-worktree provisioning was attempted as required, but the primary
checkout currently lacks the pinned clean pdfLaTeX format pair named by
`scripts/provision.py`. The already provisioned slot contained every asset
used by the focused run and the complete `tex-command` suite above.
