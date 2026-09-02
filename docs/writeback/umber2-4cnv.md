# `umber2-4cnv`: in-place durable box ownership

## Decision

Accepted. `DurableBoxState` no longer transports a 728-byte optional closure
owner through cells, group saves, checkpoint rows, and operation journals.
One state-owned reusable slot store now owns each live `DurableNodeClosure`;
all reversible lanes swap compact `(slot, incarnation)` ids. Unique box-to-page
movement empties the authoritative slot in place. Commit retires its reserved
slot and rollback reconstructs the exact loan into that slot before restoring
the cell id. Incarnations reject stale reuse.

The store is not another box or node representation. Its slot contains the
existing exclusive paired node/annex closure, grows only to the concurrent
owner high water, and reuses retired slots. There is no per-operation heap
allocation, relocation scan, journal payload clone, or replacement memmove.
Explicit TeX copies and checkpoint/group fanout remain destination-constructed
semantic copies because those histories require distinct owners.

## Authenticated 50-million-command row

The locked arXiv `2606.12566` row reused source SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
ordered 123-key closure SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`,
source epoch `1787080434`, distribution aHash64 `df66c327ae636145`, the
checked copy probe, 50,000,000 command fuel, 100,000,000 executor steps, 90
seconds, and 1,536 MiB aggregate RSS. Expected status 1 preserved the exact
semantic vector `(50000000,49903532,9457781,15936698,35326903,4203)` with
empty stdout and no PDF artifact.

The six authenticated predecessor rows in `commit_operation` and
`install_mutation` totaled 243,505 calls / 176,940,024 bytes. Complete current
symbolization contains neither frame. The only new durable-owner row is the
single authoritative `DurableOwnerStore::insert` boundary at 25,629 calls /
18,657,912 bytes, rather than repeated journal transport.

| API              |         `.5.7` calls / bytes |        Current calls / bytes |        Change calls / bytes |
| ---------------- | ---------------------------: | ---------------------------: | --------------------------: |
| `memcpy`         | `10,342,684 / 1,501,080,223` |  `9,173,529 / 1,238,030,299` | `-1,169,155 / -263,049,924` |
| `memmove`        |         `13,846 / 2,649,966` |         `21,610 / 2,827,518` |         `+7,764 / +177,552` |
| Joint            | `10,356,530 / 1,503,730,189` |  `9,195,139 / 1,240,857,817` | `-1,161,391 / -262,872,372` |
| Named allocation | `3,363,144 / 27,068,568,990` | `3,346,017 / 26,510,056,981` |    `-17,127 / -558,512,009` |

The small memmove increase has no durable-owner frame. Against the immediate
`.5.9` ownership baseline, named allocation changed by +4 calls /
-21,525,648 requested bytes and RSS fell from 168,732 to 168,448 KiB. Node and
annex peaks remain exactly 68 and 80 physical blocks, matching `.5.9`; the
removed bytes did not move into allocation, resident memory, or another public
copy API.

## Correctness, portability, and evidence

Focused `tex-state` and `tex-exec` suites pass, covering local/global group
restore, checkpoint accept/reject, operation rollback, box move/copy, loan
rollback, and stale node-region coordinates. The authenticated production row
adds exact incremental checkpoint and semantic evidence. The rebuilt Wasm
package completed the self-contained 4,000-rule-paragraph editor workload at
`stable`. Linear memory was 5,308,416 bytes before session construction,
5,701,632 after construction, and 100,859,904 after compilation and disposal:
growth is 95,551,488 bytes or 1,458 Wasm pages, versus the preserved `.5.7`
623,771,648 bytes / 9,518 pages. `scripts/check.sh` passed all gates.

Ignored evidence is under `target/umber2-4cnv/evidence/`. The profiling binary,
raw copy report, complete symbolization, engine stderr, timing, and Wasm memory
SHA-256 values are respectively
`84732cf4b6a608d368122a1f40670abb19d1ef3a31d6dbc36e305913e761b614`,
`acecbd8ac7d987fe1bead5da34c95e1b10a083bbf234883d948a1bbfeccc7dd9`,
`3de8e6124a28e99dc8d0cf6a1fbb9098ec3147b8d7996bde71721eed5d2c4601`,
`3ec91ddef66780bd8ca6337dab3367433ef4f6dba322722c594a46c70f7ed104`,
`ee5724819e046d583ad1b0f6a95aeebe385d20cbf4c3c0cb1124730dd9f55d8d`,
and `5c6e48314f0ff09cb01185425f895597fad855d480b367e7fdf9cf8cdc2d044d`.
