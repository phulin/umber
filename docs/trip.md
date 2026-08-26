# Knuth TRIP Harness

Status: manual two-phase semantic and output conformance gates.

The TeX82 TRIP and e-TeX V2 e-TRIP tests are pinned separately from the
external document corpus. Acquisition only materializes and verifies their
inputs and local oracles:

```bash
python3 scripts/provision.py worktree .
python3 scripts/provision.py worktree . --offline
```

Provisioning is not a correctness gate. The maintained conformance owners are
the exact ignored Cargo tests:

```bash
CARGO_INCREMENTAL=0 cargo test -q -p umber --test it e2e_conformance::e2e_conformance_trip_canonical -- --exact --ignored --nocapture
CARGO_INCREMENTAL=0 cargo test -q -p umber --test it e2e_conformance::e2e_conformance_etrip -- --exact --ignored --nocapture
CARGO_INCREMENTAL=0 cargo test -q -p umber --test it e2e_conformance::e2e_conformance_gentle_canonical -- --exact --ignored --nocapture
```

These full documents are intentionally ignored and never run in the routine
native suite. The format-loaded TRIP path
contains a deliberate nested `\message` construction at line 419: `\the` of
a token register must stay unexpanded while the complete message text is being
expanded, as in TeX82's `scan_toks(..., xpand=true)`. Expanding that replay a
second time recursively nests `\message{` and is an allocation-safety bug.
The same bounded replay keeps an engine-owned frozen sentinel below the raw
general text. Operand scanners may therefore read past a trailing value such
as `\the\count15` without exposing caller input, while the rendered digits are
still collected into the mark or definition. TRIP page 3 exercises this with
the numeric marks created by `\everypar`.

TRIP line 436 also exercises both branches of TeX82 §1113's explicit
discretionary hyphen construction. An out-of-range `\hyphenchar` leaves the
pre-break list empty without a warning; an in-range character missing from the
current font goes through §581's `new_character`, emits `char_warning`, and
still leaves that list empty.

`python3 scripts/provision.py worktree .` fetches official CTAN bytes into
gitignored `third_party/trip/` and verifies their SHA-256 hashes. The ignored
`e2e_conformance_trip_canonical` Cargo integration probe reaches its
registered assets through the conformance gate, which fails rather than
silently skipping when an asset is absent. It uses retained
`EngineSession`, `World` roots, and typed resource fulfillment for
format creation and the format-loaded run, with no alternate command/input
fallback. The shared conformance library gates the pinned semantic channels
and retains advisory geometry comparison in both phases. Successful recipe-owned `\dump`
construction is a structured integration gate, so its allocator, string-pool,
and serialization diagnostics are not terminal or log output-parity channels.
The harness does not normalize, spoof, or reconstruct those diagnostics. It
still requires exact construction command streams, deterministic
schema-valid publication, runtime-state exclusion, registry reconstruction,
and successful reload. Geometry differences remain visible and countable in a
non-gating report. The loaded job resumes exact semantic, terminal, log,
status, effects, and normalized-DVI comparison. The sole typed text exception
is TeX82 §638's complete `Memory usage before: ...` shipout record: its
variable-node, dynamic-node, and free-memory counts expose the reference
allocator rather than document semantics, so the comparator retains and
reports those records as advisory while comparing all surrounding text in
exact order. The e-TRIP loaded-log gate additionally projects the seven
terminal engine-usage lines (string, character, memory, control-sequence,
font-table, hyphenation, and stack occupancy) to typed labels after the
official e-TRIP artifact comparator has accepted the same surrounding log.
Those values describe WEB versus Umber physical storage; the heading, order,
fonts/pages, and every neighboring byte remain exact. It requires byte-identical final DVI
against the gitignored, locally generated
`tests/corpus/e2e/trip.expected.dvi` oracle after normalizing only the preamble
comment. The two-phase format-image path also asserts through the format schema
APIs that diagnostic and macro-invocation provenance, host effects and
capabilities, checkpoints, state-hash caches, and job journals are not durable
format state. It separately proves that loading reconstructs the selected
engine's primitive registry and frozen primitive meanings without overwriting
the format's live control-sequence meanings. DVItype is diagnostic for Umber.
Fixture regeneration independently executes both TRIP phases with
pdfTeX and installs that locally generated DVI through
`scripts/regen-fixtures.sh`; it never copies the official third-party DVI.
The official `tripin.log`, `trip.log`, `trip.fot`, and `tripos.tex` remain
pinned diagnostic references; they do not affect the current acceptance gate.
Official `trip.typ` is likewise retained and identity-pinned, but it is
DVItype's textual rendering of `trip.dvi`, not output produced by Umber. The
upstream Web2C harness compares it only after applying a platform-numeric
tolerance filter. Umber therefore keeps normalized byte-exact DVI as the
stronger sole output authority and does not duplicate that evidence with a
filtered DVItype-text gate. DVItype remains available for mismatch diagnosis.

The shared comparator's focused unit coverage remains part of the routine
suite; it is not a separate parity process or acceptance authority. The
TeX82/e-TeX observer scripts remain separate diagnostic-oracle
repeatability/transparency checks; they do not own Umber conformance.

Fixture publication is a distinct authority operation. Regenerate the local
reference-engine DVI oracles only with
`scripts/regen-fixtures.sh --case e2e/trip`,
`scripts/regen-fixtures.sh --case e2e/etrip`, or
`scripts/regen-fixtures.sh --case e2e/gentle`; regeneration is not part of an
audit run.

## Source Pins

The source of record is the CTAN `systems/knuth/dist/tex` distribution. The
manifest records ordered concrete CTAN mirror locators so one unavailable
mirror does not prevent acquisition. A locator never establishes identity:
each candidate must match the entry SHA-256 before it is accepted.

| File          | SHA-256                                                            |
| ------------- | ------------------------------------------------------------------ |
| `trip.tex`    | `15f15c2ca1470085299056ec89dea5f51e9fe9303ef25581b2f2eaf7809ae97b` |
| `trip.tfm`    | `2c94bdba9c769e885f357823a183aaa5d2267731075f040f2a03cf6442a26181` |
| `tripin.log`  | `ba01328756a8901d7c38162c9012014e9540322bf0963e105286f2a6ccb494cc` |
| `trip.log`    | `61a653523bdccab9fd3f9aa61d170d0198c322c951938327b7daef9b70f26d8b` |
| `trip.fot`    | `89e275ac12d025c06022e8dd6eb556b765954af2654b39ac2fbd451cf631b370` |
| `trip.typ`    | `64efc62b962c592c2973f8c45a78e9e5d473f8b9da53ee53bc275a98041675cc` |
| `trip.dvi`    | `09802695e330d34acec9192c15debe2de65e34fcbd3f947db9c8924240b1fe0a` |
| `tripos.tex`  | `ea7447c7a8f2de278d2f84474f22c48c9d8a0059d7e16edd578d0bbe7077b47f` |
| `tripman.tex` | `a3e47254ad87fc3fdba210d61764c93b021740f56465971f5a41103405add48b` |

The exact ordered locators live in `tests/trip-manifest.txt` beside the matching
hashes.

The locally generated `tests/corpus/e2e/trip.expected.dvi` is not the official
`trip.dvi` above. It is generated locally from the pinned `trip.tex` and
`trip.tfm` by pdfTeX 3.141592653-2.6-1.40.29 (TeX Live 2026), using the
two-phase INITEX/format-loaded workflow. Its raw SHA-256 is
`a48cec413b485403e11d35e24122aa747b3e3863a151c257fcec026580a78bf9`;
after preamble-comment normalization it is
`6420f3461dec8e5feed4b03bfc3717d00c8a36fae4fe9226f6d53a4db7592bb9`.
Regenerate it with `scripts/regen-fixtures.sh --case e2e/trip`, setting
`UMBER_REF_PDFTEX` when pdfTeX is not on `PATH`.

## Official e-TRIP Artifact Conformance

The same pinned manifest and acquisition path fetches the official e-TeX V2
e-TRIP source and the `etripin.log`, `etrip.log`, `etrip.fot`, `etrip.typ`, and
`etrip.out` masters. The harness reuses `trip.tfm` directly, as the e-TRIP
manual states that `etrip.pl` is a copy of `trip.pl`.
`scripts/regen-fixtures.sh --case e2e/etrip` creates a renamed local e-TeX 2.6
adaptation of the official 2.0 source and generates the gitignored DVI oracle
with pdfTeX. The `e2e_conformance_etrip` Cargo test first requires Umber to
match the pinned e-TeX 2.6 semantic, terminal, log, and DVI channels exactly;
the DVI comparison still normalizes only its preamble comment.

The same test then binds that exact run to the official V2 masters. It
requires exact `etrip.out` bytes. It projects the official DVItype master and
Umber DVI into numerator, denominator, magnification, page/count-register,
EOP, postamble, maximum-dimension, stack-depth, and page-count fields. This
omits only the DVItype executable banner, selected-option rendering,
floating-point pixels-per-unit value, and DVI preamble comment that the e-TRIP
manual declares platform-dependent; Cargo tests never invoke host TeXware.

Text comparison converts CTAN CRLF to LF, removes engine/startup framing and
local `./` path spelling, accounts exactly for the two licensing comments
inserted ahead of the renamed source, and applies only the e-TRIP manual's
listed date, glue-set rounding, string/control-sequence/capacity, and memory
statistics allowances. The V2 master is additionally bridged to the selected
e-TeX 2.6 profile at three explicit sites: version announcements, the changed
`this will begin denominator of:` diagnostic, and the sparse-token-register
reassignment trace. Those profile bridges cannot hide an Umber divergence:
the unnormalized e-TeX 2.6 oracle channels gate first. Any other byte differs
actionably. Focused negative controls perturb an output byte and a DVItype
page offset and require both comparisons to fail with the channel and first
offset.

The special reference engine comes from the TeX Live 2026 source snapshot
`texlive-20250308-source.tar.xz`, fetched from the University of Utah historic
archive and pinned by SHA-512
`0837c935488b96cfc8dd79f1298f283b467ab68b4163cee9cb04b79e80195982fdc5ae8a80058dc7d3e99206bfda8b3bdd11340425b08f60cbef70d5a0e22702`.
`tests/trip-reference-manifest.txt` additionally pins the extracted `tex.web`,
Web2C change inputs, TRIP configuration, and upstream TRIP harness by SHA-256.
The build records the exact configure/make commands and platform-specific tool
hashes in `target/trip-initex/build-record.txt`.

## Reference Toolchain

The current Cargo DVI gate does not require TeXware or Knuth's special TRIP
INITEX build. `scripts/build-trip-initex.sh` retains the hash-pinned Appendix A
toolchain for diagnostic transcript work; it writes provenance and wrappers
under `target/trip-initex/`.

Umber's final-DVI oracle normalizes only the preamble comment and otherwise
requires byte identity. Any mismatch writes byte, page, opcode, and
disassembly context under `target/conformance-triage/trip/`.

## Bounded mismatch triage

TRIP-specific mismatch reporting additionally writes
`target/conformance-triage/<trip|etrip>/trip-triage-v1.txt`. The report is a
deterministic, line-oriented schema `umber.trip-triage.v1` artifact, capped at
8 KiB. It records the phase, both source names and content identities, and
SHA-256 identities for canonical command events, terminal transcript, log, and
preamble-comment-normalized DVI. It never copies an output channel, an event
stream, or an unbounded log into the artifact.

Gating channels are compared in semantic diagnostic order: canonical schema-v1
command events (including manifest/header identity), transcript, log, then
normalized DVI. The identity-separated schema-v2 geometry stream is compared
and counted independently as advisory diagnostics. Complete TeX82 §638
shipout memory-usage records are likewise counted independently as advisory;
memory-like arbitrary text and every neighboring diagnostic remain byte-exact.
For e-TRIP only, the separately documented terminal engine-usage projection is
applied to the loaded log after official artifact parity.
Transcript
and log are present only for output-producing phases; successful recipe-owned
dump construction has no textual output-parity channels. Geometry
contains ordered `hpack`, `vpack`, and `shipout` records; shipout includes
TeX82 section 617's exact `count0..count9` BOP registers. INITEX records DVI as
explicitly absent rather than feeding an empty byte slice to the DVI
normalizer. `earliest.channel`, `earliest.position`,
`earliest.expected`, and `earliest.actual` describe the first gating difference,
or the first geometry difference when geometry is the only mismatch. The
report labels geometry policy as advisory and non-gating. Event context is a serialized `NormalizedEvent`; textual and
DVI context is escaped and bounded, and EOF is explicit. This makes a report
reproducible from the same comparison inputs while preserving the distinction
between a command-semantic failure and later output evidence.

The TeX82 and e-TeX observer scripts publish deterministic two-phase command-v1,
geometry-v2, transcript, log, and loaded-format DVI diagnostic channels under
`target/trip-observer-output/<trip|etrip>/`. Provisioned, lock-verified inputs
remain immutable under `target/trip-oracles/<trip|etrip>/`; observer runs never
publish into that namespace. The in-process harness captures both event
channels from one ordinary `EngineSession::run_with_observer` call;
the observer does not use replay or a legacy input-stack projection. A
successful comparison removes any stale TRIP-specific artifact and
emits no new triage output.
Each observer stages every channel beside its destination, seals it mode
`0444`, and atomically replaces the previous artifact. A rerun replaces both a
sealed destination and the private partial staging file left by an interrupted
publisher, without prompts or target cleanup. The hermetic observer ownership
self-test also hashes a synthetic locked input across two generated
publications, proving the namespaces do not alias.

Umber's detached semantic and geometry evidence uses
`tex_oracle::OracleBundle`. The oracle crate owns its `UMBREVID` encoding,
canonical pinned-header JSONL projection, independent channel sequences, and
hard limits. The format cache, command-stream façade, and TRIP/parity paths no
longer carry a second evidence codec.

## Umber format images

`umber run INPUT --format-out NAME.fmt` writes a format when INPUT terminates
with `\dump`; `umber run INPUT --format NAME.fmt` starts from that image. The
schema-12 format has an explicit fixed-width little-endian header and section
directory, compatibility fingerprints, deterministic alignment, and a
whole-image checksum. Its deterministic fixed sections contain semantic
engine state only: control-sequence namespaces and meanings, immutable
token/macro/glue/font and hyphenation content, code tables, environment cells,
and frozen node graphs. Loading validates and directly installs immutable
bases plus mutable job overlays; it never restores host pointers, hash-table
layout, allocation capacities, journals, checkpoints, input cursors,
provenance caches, or `World` effects. The official two-phase TRIP workload
exercises this format path before DVI comparison. State tests exercise the
same schema-12 encoder and decoder directly, including malformed-section
rejection, rollback, and byte-identical canonical redumps. Schema 9 images are
rejected and regenerated from source; the durable container and frozen-store migration are specified in
[frozen_format.md](frozen_format.md).
