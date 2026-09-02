# `umber2-66p0.8.40.113.5.5`: compact-node production census

## Decision

Production approval is **rejected** at commit
`a094b264a8bc93c757a822ab00b17e0b154082b0`. The required historical
`ChunkStorage<Node>::release_lineage` row of 3,143,705 calls and 528,142,440
bytes is absent, but much larger complete-node projection and reconstruction
traffic replaced it. No architecture change belongs in this assessment.
Follow-up `umber2-66p0.8.40.113.5.7` owns the one demonstrated defect.

## Authenticated 50-million-command row

One engine-entering row used the profiling binary SHA-256
`eedd85fb283c65ffb4c69078d6592bfc6b3833007695840a6985ce6d99d1c8e8`
with ELF build ID `fed3014c74a3c36bddc37e11a703ee68343b3687` and the checked
public-copy probe. The source was arXiv `2606.12566` `ArXiv.tex`, SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`.
The schema-12 format SHA-256 was
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`;
the ordered 123-key closure SHA-256 was
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`;
and the source epoch was `1787080434`.

The recorded pre-cutover root `721e833071d92bba` was no longer present in
the primary checkout. The run therefore used the preserved authenticated
2026-03-01 root whose manifest SHA-256 is
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`
and whose aHash64 is `df66c327ae636145`. Source, format, and ordered input
closure remain exact; no shared evidence was regenerated or modified.

The guards were 50,000,000 canonical-command fuel, 100,000,000 executor
steps, 90 seconds, and 1,536 MiB aggregate RSS. Expected status 1 occurred at
`(50000000,49903532,9457781,15936698,35326903,4203)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Standard output was empty and no PDF was published.

## Public-copy result

| API       |     Baseline calls / bytes |       Compact calls / bytes |          Change calls / bytes |
| --------- | -------------------------: | --------------------------: | ----------------------------: |
| `memcpy`  | 13,581,465 / 2,026,475,309 | 92,910,194 / 15,311,651,310 | +79,328,729 / +13,285,176,001 |
| `memmove` |       191,437 / 32,922,922 |          13,974 / 2,677,614 |        -177,463 / -30,245,308 |
| Joint     | 13,772,902 / 2,059,398,231 | 92,924,168 / 15,314,328,924 | +79,151,266 / +13,254,930,693 |

Both API caller sums reconcile exactly. `memcpy` and `memmove` report zero
overflow calls, overflow bytes, and probe-internal calls. The old release row
is exactly zero, and public `memmove` falls by 91.87% in bytes. That local
success cannot be accepted because total public-copy bytes rise by 643.63%.

The two leading replacement rows are exact copies of complete by-value
carriers:

| Owner                                              |      Calls |         Bytes | Bytes/call |
| -------------------------------------------------- | ---------: | ------------: | ---------: |
| `PageMaterialArena::span_chunk_node`               | 32,595,044 | 5,410,777,304 |        166 |
| `PageMaterialArena::append_reencoded_chunk_range`  | 32,273,344 | 5,389,648,448 |        167 |
| `node_region::copy_list_recursive` associated rows |  8,243,240 | 1,380,136,788 |      mixed |

The first two rows alone are 10,800,425,752 bytes, 70.54% of all current
`memcpy` bytes and 20.45 times the deleted release traffic. Source inspection
confirms that `span_chunk_node` decodes a compact record into a complete
`OwnedPageMaterialNode` return value, `append_reencoded_chunk_range` moves it
through `Result`, and `copy_list_recursive` collects complete decoded nodes
into a `Vec` before destination publication. This is the concrete rejected
hotspot; it is not allocator, annex, token, fork-tail, or libc-only traffic.

## Allocation, residency, and structural controls

The same 50M process reported 7,596,254 named allocation calls requesting
28,389,785,396 bytes: delivery/scan 414,698 / 8,885,269,108; semantic apply
6,998,144 / 2,460,283,921; evidence 3,670 / 1,569,635; cold materialization
179,077 / 17,040,994,012; and attempt scratch 665 / 1,668,720. Generation
boundary and arena growth were zero. The historical 50M copy authority did
not publish this named allocation census, so there is no same-fuel allocation
delta to invent. The nearest pre-cutover 20M census reported 1,020,769 calls /
25,956,433,040 requested bytes and is retained only as a different-length
reference.

Peak RSS was 692,640 KiB versus the historical 50M probe row's 285,464 KiB,
an increase of 407,176 KiB or 142.64%. Both are observer rows, not unprobed
latency controls. The current profiler does not publish final node-pool live
versus reusable capacity, and this rejected assessment does not add a second
instrumentation change after the exact copy hotspot is known. Existing
production controls still prove fixed pool high water across 256 unretained
pages and exact rootless suffix reclamation without source-node copy.

Focused dense-arena tests pass all 19 cases. They retain the exact 65,504-byte
2,047-record node tail, 65,532-byte 16,383-word annex tail, distinct live and
vacant flat-table entries/bytes, and zero payload copy at acceptance and
rejection. The node-token key remains 24 bytes, alignment 4, and no-Drop; no
token-store or annex owner appears in the 50M symbolized copy report.
Production candidate settlement separately reports zero allocation bytes,
zero page copies, and zero capture/accept/canonical-lane scans for both accept
and reject.

## Semantic and portability status

The prior 50M vector was
`(50000000,49911858,9459678,15939192,35332486,4203)`; current deltas after
intervening integrated semantic work are `(0,-8326,-1897,-2494,-5583,0)`.
Both rows stop at the same canonical fuel boundary with no partial artifact.
The nine compact record/annex round trips and the paired node/annex settlement
control pass, and the completed cutover's exact DVI/PDF and native semantic
gates remain the acceptance authority; no semantic defect was demonstrated by
this assessment.

The current Wasm package executes the self-contained 4,000-rule-paragraph
editor workload to `stable`. Linear memory is 5,308,416 bytes before session
construction, 5,701,632 after construction, and 181,010,432 after compilation
and disposal: growth is exactly 175,702,016 bytes or 2,681 Wasm pages. No
pre-cutover reading exists for a valid delta. The package build completed the
`wasm32` Rust compilation and reached `wasm-opt`; the separately executed
package supplied the high-water result above.

## Evidence

Ignored evidence is under `target/umber2-66p0.8.40.113.5.5/evidence/`. Raw
copy data, complete symbolization, timing, and engine stderr have SHA-256
values `8a608bbb14b8fcbff9858c5b426186d1d30bd4fae3a78caf5fc7346744de9693`,
`e617c5f134e8a63b39d0a7ac7d7244c4b8be1e1c63c32a0ca803bfc841dd1b32`,
`924f4df9d3756536ff4b25a7e023449328387bbd5dc57ac9892389a811d923c1`,
and `be6fe00d4f6b963e1e4ea782549a6e7bd61b992c47e180d4c052a6416c075a5b`.
