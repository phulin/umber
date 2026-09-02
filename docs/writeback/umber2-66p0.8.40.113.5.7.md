# `umber2-66p0.8.40.113.5.7`: borrowed compact-node traversal

## Decision

Accepted. Page-material traversal now borrows the resident 32-byte record and
its admitted typed annex, same-region reconstruction republishes directly into
the destination builder, and recursive TeX copy retains only child
coordinates before destination-directed construction. The former complete
`OwnedPageMaterialNode`/`NodeView` return, re-encode, and `Vec<Node>` copy
carriers no longer exist.

## Architecture

`PageMaterialNodeRef` is a borrow-scoped projection containing a record
reference and `NodeAnnexView`. Its public operations decode only the fields
needed by OpenType collection, hyphenation, glue diagnostics, or math lowering.
Compact-record semantic identity is computed directly from validated record
and annex fields rather than through an owned `Node`.

Range reconstruction copies the 32-byte source record, republishes its exact
fixed or dynamic typed annex payload, rewrites child coordinates, and appends
the result into the already-open destination list. Cross-region structural
copy first collects only `PageListId` children, recursively constructs them,
then performs the same direct record-and-annex publication. It carries no
complete node vector and records the original explicit TeX-copy count without
charging a second generic payload-copy API. The resident record remains 32
bytes; the existing annex remains the only payload representation.

## Authenticated 50-million-command row

One public-copy row used profiling binary SHA-256
`31277fcaa2b39e05b4e11ebbdeca0684c9dd957674d92897e352d702abfb853e`
and the checked copy-attribution probe. It reused the predecessor's exact
arXiv `2606.12566` source `ArXiv.tex`, SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`;
schema-12 format SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`;
ordered 123-key closure SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`;
source epoch `1787080434`; and preserved 2026-03-01 distribution manifest
SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`
with aHash64 `df66c327ae636145`.

The guards remained 50,000,000 canonical-command fuel, 100,000,000 executor
steps, 90 seconds, and 1,536 MiB aggregate RSS. Expected status 1 occurred at
the exact predecessor vector
`(50000000,49903532,9457781,15936698,35326903,4203)`. Standard output was
empty and no PDF was published.

## Copy, allocation, and residency result

| API       |     Predecessor calls / bytes |        Current calls / bytes |       Reduction calls / bytes |
| --------- | ----------------------------: | ---------------------------: | ----------------------------: |
| `memcpy`  | `92,910,194 / 15,311,651,310` | `10,342,684 / 1,501,080,223` | `82,567,510 / 13,810,571,087` |
| `memmove` |          `13,974 / 2,677,614` |         `13,846 / 2,649,966` |                `128 / 27,648` |
| Joint     | `92,924,168 / 15,314,328,924` | `10,356,530 / 1,503,730,189` | `82,567,638 / 13,810,598,735` |

Joint public-copy calls fell 88.85% and bytes fell 90.18%. Complete
symbolization of all 1,459 `memcpy` bins contains no
`span_chunk_node`, `append_reencoded_chunk_range`, or `copy_list_recursive`
frame. The largest remaining `memcpy` row is 71,746,752 bytes, 75.1 times
smaller than the lesser former 5,389,648,448-byte carrier row. The direct
record re-encode and child visitor appear only in sub-megabyte rows; no
replacement annex, token, allocation-owner, or scan family is comparable.
Both API totals reconcile exactly, with zero overflow bytes and zero
probe-internal calls.

Named allocations fell from 7,596,254 calls / 28,389,785,396 requested bytes
to 3,363,144 / 27,068,568,990. In particular semantic-apply allocation fell
from 6,998,144 / 2,460,283,921 to 2,765,037 / 1,139,068,411; delivery/scan and
attempt scratch are exact, while cold materialization differs by only three
calls / 896 bytes. Peak RSS was 692,328 KiB, 312 KiB below the predecessor's
692,640 KiB. No work moved into generation-boundary or arena-growth allocation,
which remain zero.

## Correctness and portability

Focused compact-record semantic tests cover every node and whatsit variant.
The 19 node-region tests and 29 page-arena tests preserve exact recursive-copy,
typed-annex republishing, dependency-floor, rollback, and settlement behavior;
`tex-exec` compiles with all updated borrowed projections. The authenticated
row's exact command-work vector supplies the production semantic control.

The rebuilt Wasm package completed the self-contained 4,000-rule-paragraph
editor workload at `stable`. Linear memory was 5,308,416 bytes before session
construction, 5,701,632 after construction, and 629,080,064 after compilation
and disposal: growth is 623,771,648 bytes or 9,518 Wasm pages, byte-identical
to the preserved predecessor evidence artifact.

## Evidence

Ignored evidence is under
`target/umber2-66p0.8.40.113.5.7/evidence/`. Raw copy data, complete
symbolization, timing, engine stderr, and Wasm memory have SHA-256 values
`4e3243d40021cf67145033aa1a97780f699b6fcef7e270d1085006d8c33b2f57`,
`8cec10e3ec840a61eecd675fcacccac842786726c77f1d7e00f08e269d8b9502`,
`0c84ce498dcd4bff41071b21497bbe1665370d114db3ec3db89c75694626a072`,
`f5058ba1e182fa6adc939b857c433a42231e0d097c18b0c017bba96a0cd226e8`,
and `e9c651469dff3a7d4ea718df9a92c150a2fd9e22f7a469d3e603781f979dfe16`.
