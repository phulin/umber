# Book performance refactors

Status: proposed architecture, based on a full-book capture at `23548ce50`.
Tracking: Beads `umber2-zauh`. Implementation has not been measured.

The strongest remaining opportunity is to reduce the work between a packed
input token and its consumer. The current command lane and resident runs have
removed substantial overhead, but the full book still spends nearly half its
attributed cycles in `tex-command`. Three related refactors are proposed below,
in priority order. Each changes an owning loop or representation and should
replace its predecessor as it lands.

## Measurement boundary

The workload is arXiv `2606.24937`, _The Hitchhiker's Guide to Agentic AI_.
The run starts from its 74 source files without auxiliary files, uses the
authenticated TeX Live 2025 distribution, and completes all 588 pages offline.
The locked `book.tex` SHA-256 is
`4e120d67196807d37974163f1bc8c5526f2b2df55a6c5ad0812378d3dfc9d9b2`.

The binary uses the optimized `profiling` Cargo profile, full debug information,
and frame pointers, with the production feature resolution and counters
disabled. A CPU-6 capture records user cycles at 199 Hz while hardware counters
measure the complete command. The diagnostic guards are 900 seconds, 6,144 MiB,
two billion fuel actions, and 100 million execution steps.

| Metric                       |                  Result |
| ---------------------------- | ----------------------: |
| Wall time                    |                249.00 s |
| User CPU                     |                246.02 s |
| System CPU                   |                  2.84 s |
| Peak RSS                     | 895,344 KiB / 874.4 MiB |
| User instructions            |         643,656,414,726 |
| User cycles                  |         463,905,804,814 |
| Instructions per cycle       |                   1.387 |
| Branch miss rate             |                   2.06% |
| Cycle samples / lost samples |              49,238 / 0 |

This is one diagnostic attribution run on a shared host. The original
120-second, 1,536-MiB, 500-million-fuel, ten-million-step corpus contract remains
the acceptance boundary. No matched revision comparison or consumed-work census
was collected, so these results establish neither a regression nor a speedup.

All 588 pages match the saved preceding Umber PDF exactly at 144 dpi and in
extracted text. `book.aux`, `book.toc`, and `book.out` are byte-identical. Against
the saved pdfTeX PDF, text matches throughout; pages 141, 161, 323, and 516 retain
their known exact raster differences. The reference was not recompiled here.

The full receipt, source-file hashes, binary hash, commands, logs, raw capture,
symbol tables, weighted attribution, and both consumer comparisons are under
`target/umber2-zauh/`. `receipt.json` records the actual distribution and format
hashes; the selected format differs from the historical format in the original
corpus receipt. Use this receipt when repeating this baseline.

## CPU ownership

The following buckets are disjoint. Each sample belongs to its nearest named
application owner, except that allocation/copy runtime and unresolved samples
retain their own buckets. Weights are the sampled cycle periods, rather than
equal weights for every sample. Unresolved samples include missing call stacks
and unknown addresses; zero lost samples does not imply complete symbolization.

| Owner                       | Weighted cycles |
| --------------------------- | --------------: |
| `tex-command`               |          47.70% |
| `tex-state`                 |          24.86% |
| `tex-exec`                  |          13.57% |
| `tex-out`                   |           3.36% |
| Allocation and copy runtime |           0.98% |
| Other named owners/runtime  |           1.73% |
| Unresolved                  |           7.80% |

The useful loop boundaries are below. Inclusive percentages overlap and must
not be added as predicted savings.

| Boundary                                         | Inclusive |  Self |
| ------------------------------------------------ | --------: | ----: |
| Expanded delivery, `expanded_next_hot_with_mode` |    32.78% | 5.37% |
| Lane delivery, `lane_command_into`               |    31.30% | 1.45% |
| Token-list scanning, `scan_toks_buffers`         |    11.38% | 0.95% |
| Macro argument matching, `macro_call_scalar`     |     8.03% | 1.56% |
| Compact raw delivery, `get_next_hot_into`        |     6.02% | 4.82% |
| Conditional skipping, `pass_text`                |     4.08% | 1.64% |
| Control-sequence name expansion, `expand_csname` |     3.89% | 0.06% |
| Recursive list copy, `CopyContext::copy_list`    |     0.91% | 0.03% |

The union of all sampled `CopyContext` methods is 0.95%. The union of token-list
scanning and macro argument matching is 18.46%, with 0.95% overlap between those
two boundaries. These unions count each sample once, including recursive calls.

Command processing remains substantial in every elapsed-time quarter. Its
disjoint attribution is about 56%, 51%, 44%, and 40%; output work becomes
visible in the last quarter. The old copy-heavy profile is therefore a poor
basis for choosing the next large refactor after
[shared box closures](shared_box_closures.md).

## First: compact input execution machine

Refactor the physical implementation beneath the
[command delivery kernel](command_delivery_kernel.md). Keep the canonical
semantic input stack, and make its ordinary path execute directly over a small
frame and a word-and-meaning result.

Today `read_resident_run` selects a storage variant, produces a `ResidentWord`,
and hands it through reading, resolution, materialization, settlement, and
classification. Scalar callers traverse these stages for each word.
`HotCommand` still carries delivery coordinates, source facts, alignment flags,
and a font capability even when a scanner needs only a digit or delimiter.
Macro admission also returns an owned body that activation moves through a
cursor, resident row, and `InputLevel` before inserting it into the stack.

The replacement has three boundaries:

- Definition admission returns a small copyable header. Activation constructs
  the final body frame in its stack slot, acquiring its storage owner once.
  Empty parameterless macros finish directly from the header.
- The frame retains position, limit, physical storage coordinate, behavior,
  and rollback identity. Invocation metadata and diagnostics have a cold owner
  addressed by that identity. Retirement releases the frame's storage once.
- Reading resolves a live meaning once and returns the facts the consumer
  needs. Scalar scanners and expansion classify directly. A command with full
  delivery authority is materialized when execution, backup, observation,
  alignment interception, or recovery requires it.

Keep the exposed reader selected across an ordinary run. Parameters and frame
exhaustion settle against that same stack and resume the run. Source refill,
resource misses, and exceptional settlement use the canonical cold boundaries.
Control-sequence meanings remain live eqtb reads; compiling a macro body must
never freeze the meanings of its control sequences.

The expanded loop, compact raw loop, lane delivery, stack push, and ordinary
retirement alone account for about 14.3% disjoint self cycles. Definition
admission, chunk handles, and caller-side materialization add further costs.
Removing half of that 14.3% would save about 7.2% of total cycles. A useful
ambitious acceptance target is 15–25% fewer full-book instructions across the
complete redesign, with a separately demonstrated cycle reduction. That is a
target, not a measured forecast. Merely halving all `tex-command` work would
save 23.8% of total cycles, or about 1.31× overall under Amdahl's law.

## Second: definition-owned matcher plans and fused token collectors

Move immutable matching work to definition publication, then let matching and
collection consume packed spans through the same input execution machine.

`MacroParameterPattern` already stores parameter offsets. However,
`scan_delimited_argument` rebuilds KMP failure links for every invocation and
returns delimiter candidates to scalar command delivery. Expanded
`collect_replacement` likewise performs raw command delivery, expansion-route
classification, expansion, collector classification, and append as separate
stages for each word. Unexpanded definitions already have direct resident runs;
the expanded collector is the larger remaining integration opportunity.

Publish a definition-owned matching plan with leading-pattern bounds,
undelimited slots, delimiter words, and failure links. Build it once when a
definition becomes immutable, including format loading. Matching executes that
plan against the current reader, retaining only its prefix state, brace depth,
and argument writer. A matched delimiter advances input without entering an
argument sink or constructing a delivered command. Outer validity, paragraph
recovery, and the special opening-brace delimiter still settle in canonical
order.

The expanded collector supplies its grammar to the existing expansion loop:
ordinary expanded words append directly; `\the`, `\unexpanded`, and
`\detokenize` retain their special splice rules; closing braces remain owned
by the collector. Definition publication consumes the writer's finished spans
directly. Matching and collection keep their distinct TeX grammars while sharing
the reader and expansion implementation.

The measured 11.38% collector and 8.03% matcher boundaries justify this work,
but they include real scanning, storage, and expansion. Halving the collector
boundary alone would save at most about 5.7% overall. Failure-link compilation
is one concrete removed cost; its individual share has not been measured.
After the first refactor, collect a new profile before assigning a combined
savings target to this one.

## Third: direct scanner-to-executor operation path

Extend the [command lane](main_control_dispatch.md) into the common group, box, and
mode-specific operation paths, with one shared execution implementation per
command family. A successful common command should scan directly into the
owner that applies it and settle one command-sized rollback unit.

`execute_cold_episode` accounts for 22.91% inclusive cycles and
`delivery::scan_command` for 6.61%. Much of their inclusive cost is genuine
scanning and typesetting. Their self costs are only 1.24% and 0.59%, so deleting
those wrappers alone cannot yield a large gain. The design must remove the
representations and repeated preparation beneath them.

Make each common scanner apply through the existing typed semantic owner. It
should not construct a generic cold operation and then redispatch its kind.
Capture each affected rollback component before its first mutation, keep its
mark through scanning and application, and settle it at that command's end.
Unchanged components do not reopen or recapture state. Host effects and page
publication end the admitted run through its existing boundaries.

Start with frequently entered box/group paths, then common dimension/glue
assignments. Profile each family before widening the lane further. Keep full
preparation for diagnostics, tracing, observation, and uncommon commands.
Earlier settled commands must survive a later command's recoverable error;
merging several commands into one rollback unit is unsound under the current
contract. The already-small state-roll boundary, 0.36% inclusive, does not
justify another journal-only redesign.

## Performance and semantic acceptance

Land the input representation and consumer changes in independently measurable
chunks, followed by matcher/collector integration and direct operation work.
Record each implementation task and its dependencies in Beads. Reprofile after
each major boundary changes; the three proposals share costs and their savings
cannot be summed from this baseline.

Use prebuilt production binaries in order-balanced comparisons with identical
source, auxiliary-file policy, format, distribution, date epoch, and guards.
Measure full-book instructions, cycles, user time, wall time, and RSS. Use
separate matched profiling-feature builds for structural/work counters. Report
exact fuel and any replay-sensitive work differences instead of treating equal
fuel as proof of equal semantic work.

Focused controls must exercise source and resident input, live meaning changes,
macro delimiter overlap, nested braces, outer and paragraph recovery,
`\noexpand`, protected definitions, alignment templates, deferred writes,
resource suspension, and exact fuel exhaustion. Ownership controls must show
that rollback restores the same cursor and releases each owner exactly once.
Use existing committed oracle fixtures; new reference fixtures are regenerated
through `scripts/regen-fixtures.sh`. Run the owning tests, the routine
`cargo test -q --tests` suite, and `scripts/check.sh` as each refactor lands.

The full-book semantic gate is exact auxiliary output and all-page text/raster
comparison. Preserve the four independently observed reference differences as
unresolved compatibility work. Performance acceptance must eventually repeat
the original corpus guards; diagnostic completion alone is insufficient.
