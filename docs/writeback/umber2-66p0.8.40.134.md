# `umber2-66p0.8.40.134`: one owner for acquired resource bytes

## Measured owner chain

The authenticated `.132` authority attributed 112,132,392 public `memcpy`
bytes to seven handoffs of the same external PDF-image payload:

| Owner                                             | Baseline calls / bytes | Final calls / bytes |
| ------------------------------------------------- | ---------------------: | ------------------: |
| `PdfState::last_external_image`                   |        21 / 28,022,514 |               0 / 0 |
| `LocalResolver::resolve`                          |        15 / 18,702,844 |               0 / 0 |
| `World::materialized_file_bytes`                  |        15 / 18,702,844 |               0 / 0 |
| `PdfState::external_image_record`                 |        14 / 18,681,676 |               0 / 0 |
| `parse_pdf_image`                                 |          7 / 9,340,838 |               0 / 0 |
| `PdfState::allocate_external_image`               |          7 / 9,340,838 |               0 / 0 |
| `CommandHostCapabilities::pdf_image` source clone |          7 / 9,340,838 |               0 / 0 |
| **Total**                                         |   **86 / 112,132,392** |           **0 / 0** |

The final exact report symbolized the top 120 owners for each public copy API.
It contains none of these functions or the external-image source clone. Both
reports reconcile their caller bins exactly and report zero overflow bytes,
overflow calls and probe-internal calls.

## Ownership change

`tex-content` now owns `SharedBytes`, a one-word immutable handle. It adopts a
fresh `Vec<u8>` without relocating the vector payload and wraps an existing
`Arc<[u8]>` without copying it. Cloning the handle increments only its compact
owner count; byte equality and hashing remain content-based.

VFS storage and resolved responses, World records, source registration,
observation, incremental registered inputs, PDF parsing, host capability
replay, the PDF payload arena, image record queries, terminal completion and
`tex-out` finalization input now carry that handle. Parsers and serializers
borrow a slice. Native acquisition therefore performs one payload allocation
and read, while WebAssembly retains the existing binary `Uint8Array` wire
boundary and adopts its decoded vector once.

The PDF payload arena stores handles behind compact payload IDs. Checkpoint,
candidate, current/prior settlement and rollback copy only arena structure and
handles. Format serialization still writes bytes, and format decode creates a
new session owner as required for format/session isolation. Imported streams
that require decompression still allocate their semantically new decoded
content; those are not clones of the acquired file owner.

## Exact authority comparison

The final run reused the `.132` 20M-action authority, explicit offline
distribution, schema-12 format, ordered prefetch closure, source-date epoch,
copy probe and `cycles:u` sampling command. It retained the expected status 1
fuel stop and the exact command-work vector. Empty stdout is byte-identical.

| Counter                       |       Baseline |          Final |                  Delta |
| ----------------------------- | -------------: | -------------: | ---------------------: |
| Whole-process `memcpy` calls  |      7,287,659 |      7,210,352 |                -77,307 |
| Whole-process `memcpy` bytes  |  1,080,804,810 |    893,028,925 | -187,775,885 (-17.37%) |
| Whole-process `memmove` calls |        238,872 |        238,988 |                   +116 |
| Whole-process `memmove` bytes |     40,776,774 |     40,785,918 |                 +9,144 |
| Hot-core allocation calls     |      1,033,433 |      1,033,576 |                   +143 |
| Hot-core requested bytes      | 26,007,521,322 | 25,951,473,870 |            -56,047,452 |
| User CPU seconds              |           6.09 |           6.14 |                  +0.05 |
| System CPU seconds            |           0.56 |           0.51 |                  -0.05 |
| Total CPU seconds             |           6.65 |           6.65 |                   0.00 |
| Wall seconds                  |           8.13 |           8.11 |                  -0.02 |
| Peak RSS                      |    231,452 KiB |    163,428 KiB |  -68,024 KiB (-29.39%) |

The total copy reduction exceeds the selected 112,132,392-byte image chain
because the same principled owner also removes full-payload `Vec` to `Arc`
conversions and re-clones for the other admitted file resources in this run.
The small `memmove` and allocation-call changes are representation/layout
effects; CPU is flat within one hundredth of a second while requested bytes
and resident memory fall materially.

## Semantics and validation

Focused tests prove that the original allocation address survives VFS/World
admission, PDF inspection, suspended host replay, PDF allocation and lookup,
last-image lookup, checkpoint/candidate ownership, detached completion and
finalization input. The synthetic external-PDF output finalizes twice to
byte-identical bytes and is accepted by the independent PDF query parser.
Existing resource-retry, format, rollback and current/prior candidate tests
remain green.

Validation passed:

- focused shared-owner, PDF-state, resolver, VFS and output tests;
- the profiling allocation checkpoint gate;
- `umber-wasm` for `wasm32-unknown-unknown`;
- the complete `cargo test -q --tests` routine suite;
- `scripts/check.sh`: dprint, Biome, rustfmt and both clippy resolutions across
  32 workspace members.

The deferred broad PDF corpus comparison was not run.

## Remaining copy owners and evidence

The largest remaining `memcpy` owner is
`ChunkStorage::release_lineage` at 2,333,768 calls and 392,073,024 bytes.
Direct line-break collection owns the next four roughly 19 MiB rows. TFM
ligature pending-character movement remains at 112,216/18,852,288 and two
82,348/13,834,464 rows. The two largest `memmove` owners remain direct
line-break collection at 115,040/19,326,720 and
`PageMaterialArena::push_active_list` at 113,493/19,066,824. None is an
external-resource payload handoff.

Ignored evidence is under `target/umber2-66p0.8.40.134/after-final/`.
Baseline/final binary SHA-256 values are
`e557e962ab167755f0b8b6c29de05e31bbcf27fad73820d1b1599d7b78556acc`
and `1a5b15c0c3e0edae90f870a56cec518a5ed9dd1e58655916a68269c6ef0992fd`.
Baseline/final symbolized copy-report SHA-256 values are
`f05ff0c0948713155a39d405526c9b6c117d6ea1f5a7a0c961a958e16e7840b4`
and `a3f7b70bb88119650994c140e2e36490069db59aa3b39fef317bcd78d6f1b1f4`.
Baseline/final `perf.data` SHA-256 values are
`25311e9dbe173750ad4585357723cefbf38fff1eca1d6b4fe2a49ff0ac1c2014`
and `cfd7e1a3ff72a867abbaea6ea627facc6b98cb6f18b7c4611a5dea33ae32f829`.
