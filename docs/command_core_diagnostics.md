# Command-core diagnostic tools

Status: operational reference for the canonical differential tracer and first-failure locator.

Two tools locate a `umber2-johp` command-core divergence or failure. This
document describes what each tool is, what it requires, and what it prints.
For the order to run them in, why the retired Umber implementation is never
an oracle, and what to do with the result, see the diagnosis order in
[Canonical Divergence Working Contract](canonical_divergence_workflow.md#2-diagnosis-order).

## Differential Tracer

```bash
cargo run-dev -q -p tex-command-stream --bin tex-command-stream -- --repository . --max-divergences 100000
cargo run-dev -q -p tex-command-stream --bin tex-command-stream -- --repository . --realign-window 128
cargo run-dev -q -p tex-command-stream --bin tex-command-stream -- --repository . --ungrouped
```

The default budget (`DEFAULT_MAX_DIVERGENCES` = 20) saturates on `gentle`, so
a run without `--max-divergences` returns `PARTIAL` (exit `2`) and totals that
are floors. Pass a budget large enough to exhaust every fixture whenever the
totals are going to be compared against anything.

Run this from the repository root. It replays the committed
`tests/corpus/command/tex82` fixture registry and the separately pinned
`tests/tex82-oracle/geometry.tex` microfixture through the instrumented command
boundary and compares the translated `tex-oracle` event streams against their
expected traces. The geometry source selects schema v2 and compares its
detached hpack, vpack, and shipout projection; ordinary command fixtures remain
schema v1. The run is fully hermetic (no external corpus, distribution, or live
TeX tool required) and never invokes a reference engine.

The native correctness suite runs this committed-microfixture comparison and
requires gating command channels to be `CLEAN`; its
`committed_tex82_command_traces_are_clean` test uses the
committed-only runner, which validates the fixture inventory before replaying
it. An absent or drifted committed fixture therefore fails explicitly rather
than making the gate look clean. Selection also enforces a structural
microfixture footprint: at most 64 source files, 64 KiB of combined source,
and 50,000 ordered events per fixture. A registry entry beyond any bound fails
with its observed footprint, every limit, and the manual command to use
instead. This is deliberately name-independent, so accidentally registering
any full document is rejected rather than relying on a list of known document
names. The generated document-trace tree is loaded only by the explicit CLI
runner, so the routine suite does not replay Plain, Story, Gentle, TRIP, or any
other full document. The committed geometry microfixture is deliberately
font-independent and covers explicit packaging, paragraph line packing, an
explicit shipment, and end-of-job page-builder shipment. Controlled hpack and
shipout mutations produce separately counted, explicitly non-gating
`geometry_mismatch` diagnostics, including both expected and actual signed
scaled-point values. Geometry collection, expected streams, and comparison
remain intact, but geometry differences never change the tracer verdict.

It reports a ranked WORKLIST, not just the first divergence:

Every run that happened prints its report, ending with a `VERDICT:` line
naming the outcome and the exit status carrying it. The status answers one
question: whether the printed totals are the whole truth.

- Exit `0` (`CLEAN`): every gating command fixture was compared to exhaustion
  and none diverged. Advisory geometry differences, if any, are counted and
  labeled separately. The report is still printed, because a check that prints
  nothing cannot be told apart from a check that did not run.
- Exit `1` (`DIVERGED`): every registered fixture was compared to exhaustion,
  so the divergence total is exact. Prints up to `--max-divergences` ordered
  divergences (default
  `DEFAULT_MAX_DIVERGENCES` = 20) per fixture, in stream order across every
  registered fixture, collapsed into one entry per root site (see "Grouped
  worklist" below). Two entry shapes share this ordered list:
  - A stream mismatch: the expected event, the actual observed event, and
    source context, labeled with a cheap structural `kind` (for example
    `command_identity_mismatch`, `command_operand_mismatch`,
    `event_kind_mismatch`, `mutation_mismatch`, `stream_truncated_early`) so
    same-kind entries can be batched without re-running the engine. The
    header line also carries the resynchronization the comparator applied and
    the cascade that resynchronization absorbed:

    ```text
    fixture <name> diverged at event 11375 (observed event 11385)
      [event_kind_mismatch] (resync: 1 oracle event(s) dropped by Umber;
       suppressed 32 cascade event(s))
    ```

    The observed index is printed only once it has drifted from the oracle
    index. See "Stream alignment" below for what each resync means.

    Both rendered events are truncated to a bounded number of characters, so
    a long payload -- a macro body, a token register, a mutation value --
    can differ past the cut and print two identical-looking sides. When that
    happens the entry carries one extra pair of lines naming the character
    offset where the two renderings first differ, with a window of context
    around it:

    ```text
    first difference at character 4325, past the truncation above:
      expected: …, OracleToken { … "mac_param" … }, OracleToken { … }] })
      actual:   …, OracleToken { … "mac_param" … }, OracleToken { … }] })
    ```

    It is text-level rather than schema-aware, so it works for every event
    kind without enumerating payload fields, and it is emitted only when the
    truncation actually hid the difference.
  - A contained replay failure (`engine panicked` or `replay failed`): a
    command-core `ExecError` or a Rust panic during that fixture's replay is
    caught (`catch_panic`/`ReplayFailure`) and reported as its own ordered
    entry with the fixture, the event index it occurred after, and the
    failure's message (a panic's message and source location, exactly as
    the default panic hook would have printed -- `RUST_BACKTRACE=1` still
    produces a full backtrace on stderr). It does not abort the run: fixture
    replay continues afterward for any fixture ordered after it, up to the
    divergence budget. A panic outside a fixture's replay proper (fixture
    loading, suite/contract validation, argument parsing) is not contained
    and still aborts the process with the ordinary Rust panic exit code.
- Exit `2` (`PARTIAL`): the run did not compare everything it registers, so
  every printed total is a LOWER BOUND rather than a total, and a total of `0`
  would not mean convergence. Two conditions earn it, and the verdict names
  which applied and to which fixtures: a registered document trace that is not
  generated on this checkout (regenerate with
  `scripts/build-tex82-document-traces.sh`), and a fixture whose
  `--max-divergences` budget stopped its comparison early (raise the budget).
  This status exists because `umber2-johp.168` found a fresh checkout
  reporting `4 ordered divergence(s)` where the true count was `160`: three of
  the four registered fixtures had never been compared, and nothing in the
  exit status distinguished that from convergence. Never rank or dispatch from
  a partial worklist.
- Exit `3`: the run could not be performed at all -- a usage error, an
  unreadable suite, or a document registry inconsistent with its committed
  pin. It is kept distinct from exit `1` so "the tool refused to run" is never
  read as "the tool ran and produced this worklist".

See `docs/command_semantic_fixtures.md` and `docs/alignment_brace_semantics.md`
for the fixture registry and event schema this replays and compares against,
and `tools/AGENTS.md` for what the tool does and does not do.

#### Grouped worklist and run accounting

One defect reaches the ordered worklist once per source position it recurs
at, so a preload loop that assigns the same wrong meaning forty-eight times is
forty-eight entries that are identical apart from their `SourceLocation`. The
report collapses those into one entry each. The run opens with two separately
labeled totals and a per-fixture accounting:

```text
759 ordered divergence(s) in 200 root site(s):
  divergence(s): what the comparator found. Grouping does not change this
    number; it is the one to compare against historical totals.
  root site(s): the entries below, one per group of divergences that are
    identical after erasing source positions and nothing else. Every
    divergence is in exactly one group; none is dropped, sampled, or
    truncated. Pass --ungrouped for one entry per divergence.
per fixture, in replay order:
  tex82/command-transitions-v1  1 divergence(s) in 1 root site(s), first at oracle event 5892
  tex82/document-plain-v1       0 divergence(s)
  tex82/document-story-v1       0 divergence(s)
  tex82/document-gentle-v1      758 divergence(s) in 199 root site(s), first at oracle event 102452
```

The two numbers answer different questions and only one of them is comparable
against a historical figure.

- **divergence(s)** is what the comparator found. Grouping does not change it,
  and every "N entries" figure recorded in `umber2-johp` before grouping
  existed is this number. Compare a before/after fix against _this_.
- **root site(s)** is how many entries the report prints. It is a triage
  metric: it says how many distinct things a coordinator has to dispatch.

Grouping is presentation only. The ordered comparison, the entry order, and
the divergence count are identical with and without it, and `--ungrouped`
prints the one-entry-per-divergence worklist -- byte-identical to the
pre-grouping report body -- so the grouped view can always be checked against
the list it summarizes. Both views print both totals.

Two divergences are the same root site when they are equal after erasing every
source position and _nothing else_. `group::positionless_event` is an
exhaustive match over the `tex-oracle` event schema -- `CanonicalCommand`'s
location plus every `OracleToken` reachable through commands, recovery events,
macro arguments, token lists, scanner results, mutations, diagnostics, and
effects -- so adding a schema variant fails to compile rather than silently
carrying a position, or a payload, into the key. Everything else separates two
entries: differing operands, differing token payloads, a differing repair
shape (three dropped oracle events is not twenty-one), a differing fixture,
and a macro call's token-list address. `Repair::AnchorResync` compares by
anchor _kind_ with a line-anchor's line erased, since that line is a position
like any other. The bias is deliberate: under-merging leaves a longer
worklist, but over-merging hides a second defect behind the first.

A grouped entry prints its count, renders its first occurrence exactly as the
ungrouped worklist renders it, and then names every occurrence:

```text
[75] x109 fixture tex82/document-gentle-v1 manifest=... diverged at event 141407 ...
  expected: Input(InputEvent { transition: Push, reason: TokenList, name: "every_par" })
  actual: Command(CommandEvent { delivery: Raw, command: CanonicalCommand { ... } })
  context: source=gentle.tex; input_level=5803; position=0
  recurrence: 109 exact occurrence(s) of this root site, 1196 suppressed cascade event(s) in total;
    the entry above is the first. Every occurrence, by oracle event index:
    141407, 141450, 141486, 141551, 141619, 143260, ...
```

The oracle event index list is printed whole and never elided: an entry that
stands for a hundred occurrences has to let the agent dispatched on it reach
all hundred. A group's suppressed cascade is the sum over its members. A
single-occurrence entry prints `x1` and no recurrence block.

Two bounds the report used to leave a reader to infer are now printed.

- A fixture whose `--max-divergences` budget was reached while events remained
  prints `BOUNDED:` under its accounting line, naming the flag and saying that
  comparison of that fixture stopped there. See "What the budget counts"
  below for the unit, which grouping does not change.
- A document trace that is not generated on this checkout is listed as
  `not compared -- trace not generated on this checkout`, in addition to the
  stderr notice, so a fixture that never ran cannot read like a clean one.
  Compared fixtures with no divergence are listed with an explicit `0`.

Both bounds also decide the exit status, because a reader who checks only
`$?` is exactly the reader a partial run misleads. Either one makes the run
`PARTIAL` (exit `2`) no matter what the divergence total says, and the report
closes with a `VERDICT:` line repeating the outcome, the status, and which
fixtures were left short:

```text
VERDICT: PARTIAL (exit 2) -- this run did not compare everything it
  registers, so 1 ordered divergence(s) is a LOWER BOUND, not a
  total, and a total of 0 would not mean convergence.
  never compared (3): plain, story, gentle
  generate the missing traces with scripts/build-tex82-document-traces.sh
```

A partial run also withdraws, in the header itself, the instruction the
complete-run header gives. The header is where a reader takes a number from,
so leaving `it is the one to compare against historical totals` printed over a
floor would be this epic's recurring defect in miniature -- a number labeled
as something it is not. A bounded or incomplete run prints instead:

```text
20 ordered divergence(s) in 7 root site(s):
  LOWER BOUND: this run stopped short of comparing everything it registers
    (the per-fixture accounting below and the VERDICT line at the end name
    which fixtures and why). Every total above is a floor, not a total, and
    none of them is comparable against a historical figure.
  divergence(s): what the comparator found before it stopped short.
    Grouping does not change this number; the bound above does.
```

An exhaustive run's report is byte-identical to what it was before that
annotation existed, so no figure measured from an exhaustive run moved.

#### What the budget counts

`--max-divergences N` bounds **ordered divergences**, per fixture. It bounds
neither root sites nor printed entries, and it never has. Since the worklist
began printing one entry per root site the three are different quantities: a
bounded run of `N` divergences prints at most `N` grouped entries and usually
fewer, and prints exactly `N` under `--ungrouped`. Re-basing the budget onto
root sites was considered and rejected (`umber2-johp.207`):

- It would bound nothing. The budget exists so one long fixture cannot produce
  an unbounded walk and an unbounded report, and the case it was introduced
  for is a single structural defect recurring without end. That defect is
  _one_ root site however many times it recurs, so a budget of 20 root sites
  would walk the whole fixture and print a recurrence index list thousands
  long -- the outcome the budget prevents today.
- It would move the ambiguity, not remove it. Budget and printed entry count
  agree exactly under `--ungrouped` today and would stop agreeing there. No
  unit equals the printed entry count in both views, so that equality is not
  an available invariant.
- It would make the comparator depend on the presentation layer. Grouping is
  documented as changing only how the worklist prints, never what is compared
  or in what order; a root-site budget would let the grouping projection
  decide where the comparison stops, so the two views would compare different
  amounts of the stream.

The invariant kept instead is that every number names its unit where it is
printed. A bounded fixture's `BOUNDED:` notice says the budget counts ordered
divergences and neither root sites nor printed entries, and names its
divergence total and its root-site total as floors:

```text
tex82/document-gentle-v1  20 divergence(s) in 7 root site(s), first at oracle event 204839
                          BOUNDED: --max-divergences 20 counts ordered divergences; it
                          counts neither root sites nor printed entries. Comparison of this
                          fixture stopped at 20 of them, so its 20 divergence(s) and
                          7 root site(s) above are both floors: more of each exist
                          beyond its last entry.
```

One divergence per fixture sits outside this budget: the contained replay
failure (`engine panicked` / `replay failed`). It is at most one entry, names
a concrete `ExecError` or panic site, and must not be crowded out by the
twentieth consecutive mismatch of an already-reported structural defect. A
bounded fixture's divergence total can therefore exceed the budget by one, and
when it does the notice says so rather than leaving the arithmetic to look
like an overrun:

```text
Its contained replay failure is reported outside the mismatch
budget, which is why 21 is more than the budget of 20.
```

#### Stream alignment

The comparator (`tools/tex-command-stream/src/compare.rs`) treats the pinned
oracle stream and the observed stream as two sequences to be aligned, not as
two index-parallel arrays. Index-aligned comparison is only correct while
both streams agree on how many events each delivery produces; one dropped or
extra event otherwise turns every later index into a mismatch, and one root
defect fills the whole per-fixture budget with entries that say nothing new.

Every event splits into an alignment KEY and a PAYLOAD.

- The key is identity: the event kind, the canonical command identity
  (command name, control-sequence spelling, raw/expanded delivery) and its
  source position, and every structural transition -- input push/retire/stop,
  condition push/branch/pop, alignment transitions, token-list
  splice/complete, macro argument vs. activation.
- The payload is content: operands, scanner results, mutation keys and
  values, align state, token lists, diagnostic arguments.

A command's source position is part of its identity because long runs of
like-catcode characters are otherwise indistinguishable, and a shifted stream
could confirm a realignment against the wrong occurrence.

From that split the comparator produces one of these resyncs per entry.

- `payload differs, streams stay aligned` -- same key, different payload. A
  content-only defect; nothing was skipped and nothing cascades.
- `N oracle event(s) dropped by Umber` -- the oracle emitted N events Umber
  never produced.
- `N extra Umber event(s)` -- Umber emitted N events the oracle never
  produced.
- `N oracle event(s) replaced by M Umber event(s)` -- a short edit run; one
  replaced by one is an ordinary substitution.
- `structural: ... rejoined at <anchor> after skipping ...` -- nothing
  confirmed inside the window; the anchor fallback rejoined the two streams.
- `structural: ... no shared anchor; comparison of this fixture stopped here`
  -- neither the window nor an anchor confirmed a repair.
- `one stream ended with N event(s) remaining in the other` -- one stream ran
  out first.

On a key mismatch the comparator runs a wavefront search confined to a window
of events on each side, visiting candidates in ascending edit distance (and,
within one distance, ascending oracle skip) so the smallest repair wins
deterministically. A candidate is accepted only after a run of consecutive
key-equal pairs confirms it. When fewer than that many pairs remain before
the end of a stream, all remaining pairs must agree; skipping both streams to
their ends with no agreeing pair at all is a fork, not a repair.

If nothing confirms inside the window, the divergence is structural and one
anchor resync is attempted: both streams are scanned forward over every
high-salience boundary -- an input-stack push/retire/stop, or the first
delivery attributed to a new source line -- inside the scan bound, and the
streams rejoin at the most identifying shared boundary that carries the same
confirmation. If that also fails, comparison of that fixture stops and says
so. The bias is deliberate: cascade noise is visible, but a real defect
hidden behind an over-eager realignment is not.

The two anchor kinds are not equally identifying, and the search ranks them
in that order rather than by cost. A shared source line names the same
physical position in the same named file on both sides, so it is evidence
that the streams are at the same point in the _document_. An input push names
only the shape of a boundary: every macro activation in a run carries the
identical `Push/Macro macro` key and every backup the identical
`Push/Backup backup`, so a shared one is evidence of nothing beyond "both
sides pushed something". A shared line therefore wins over any anonymous
boundary in reach, however much cheaper that boundary is; anonymous
boundaries are used only when no line is shared inside the scan.

Within one class, least total skip decides, and that is not a tie-break
detail either. Rejoining at a costlier shared anchor lands the streams on a
boundary they agree at only locally, and the next real key mismatch then has
no shared anchor left inside the scan, so an inherited over-costly rejoin is
reported as a structural fork that stops the fixture. Anchors are enumerated
by oracle offset, and the cheapest pair is frequently not the first one
visited: a distant oracle anchor paired with an immediate observed one
undercuts a nearby oracle anchor paired with a far observed one.

This is not a global minimum-edit-distance diff, and deliberately so. Myers
is `O(ND)` and Gentle's trace is over 100 000 events; worse, a global minimum
would happily pair oracle event 700 with observed event 40 000 when that
minimizes total edits. Every search here is local, bounded, and paid at most
once per reported divergence, so the run stays linear in the streams.

`suppressed N cascade event(s)` counts the mismatches plain index-aligned
comparison would have reported over the stream region this entry covers --
from this entry's oracle index up to the next reported entry's, or to the end
of the streams for the last entry -- not counting the entry itself. It is the
cascade the entry stands in for, and it is how to tell one root site from
many: as of this writing `gentle` reports 13 550 divergences in 2297 root
sites where index-aligned comparison would report hundreds of thousands, and
`plain` and `story` are clean.

#### Alignment tunables

Three flags bound the search. Widen them when a suspected repair is larger
than the defaults; narrow them to prove a reported realignment is not an
artifact of an over-generous bound.

- `--realign-window` (default 64) -- half-width of the wavefront search, in
  events per stream.
- `--realign-confirm` (default 8) -- consecutive key-equal pairs required to
  accept a realignment.
- `--anchor-scan` (default 4096) -- events scanned forward on each side by
  the structural anchor fallback.

A window of 64 costs `O(window^2)` key comparisons in the worst case, paid at
most once per reported divergence, and comfortably spans every repair shape
this epic has produced (a missing backup push, a duplicated raw/expanded
delivery pair, a macro activation expanded one level too far) while staying
far below the distance at which a confirmed match would be coincidence rather
than the same point in the document. A confirmation run of 8 is far more than
the two or three events that repeat by chance inside a run of like-catcode
characters, and small enough that a genuine repair immediately followed by a
second independent defect still confirms, leaving the second defect to be
reported on its own. The anchor scan is only reached once the window search
has already failed, so 4096 events -- roughly a page of document activity --
is generous on purpose. It is the only bound on the fallback's reach: every
anchor inside it is a candidate, so widening the flag really does widen the
search. A dense trace region crosses dozens of input-stack boundaries in a
few hundred events, so any secondary cap on the anchor count would quietly
shorten this flag to a fraction of its stated reach.

All three flags take a positive integer and reject anything else with a usage
error.

#### Registries

The tracer replays two registries in this order.

1. **Committed fixtures** under `tests/corpus/command/tex82` -- today the
   single synthetic `tex82/command-transitions-v1`. Always present, fully
   hermetic, font-independent.
2. **Full-document traces** under `tests/corpus/command/tex82-documents` --
   `plain` (plain-format bootstrap alone), `story`, and `gentle`, each running
   `\input plain` plus the corpus document through real INITEX TeX82. These
   are _generated on demand and gitignored_: one plain trace is about 17 MB
   and Gentle's is about 156 MB, so committing them would add roughly 190 MB
   of exactly-reproducible generated bytes to the repository. A document
   whose trace tree is absent prints a one-line skip notice on stderr and is
   not a failure, so the tracer stays usable on a checkout that has never
   built the oracle.

Both registries produce the same ordered worklist entries; document
divergences follow committed-fixture divergences in the report.

The full-document registry is a manual parity diagnostic only, not a native
test-suite gate. Run the CLI above after generating document traces when
investigating a document-level divergence; a fresh checkout reports absent
generated traces as `PARTIAL` instead of silently treating them as clean.

Once the document tier is present, run the tracer through the `test` profile
(`opt-level = 1`) rather than the plain `dev` profile -- replaying hundreds of
thousands of document events unoptimized takes minutes where the `test`
profile takes seconds:

```bash
cargo run-dev -p tex-command-stream --bin tex-command-stream -- --repository . --max-divergences 100000
```

#### Generating the full-document traces

```bash
scripts/build-tex82-oracle.sh --offline          # once, builds the pinned oracle
scripts/build-tex82-document-traces.sh           # all three documents
scripts/build-tex82-document-traces.sh --document plain   # one document
```

`scripts/build-tex82-document-traces.sh` stages
`tests/tex82-documents/<name>/root.tex`, `third_party/corpus/{plain,<name>}.tex`,
`third_party/hyphen/hyphen.tex`, and every `third_party/fonts/*.tfm` into a
fresh run directory; runs the clean and instrumented oracle executables (plus
a third repeat run) and requires their ordinary terminal/log/status/DVI
channels to agree and the instrumented trace to be bit-identical across
repeats; then derives a fully validated `tex-oracle` fixture through
`tex-oracle-bootstrap` and publishes it, with the staged TFMs beside it under
`fonts/`. It performs no network I/O. A partial (`--document`) run prints its
records instead of rewriting the contract, so the pinned file can never
describe a half-regenerated tree.

A full run stages all three fixtures before publishing them as one tree. If a
prior generated tree exists, the script moves it aside until the new tree and
committed identity contract have both been installed, then discards it. If
contract publication fails, it restores that prior tree; on a fresh checkout,
where the gitignored tree is documented to be absent, rollback restores that
absent state instead. Thus first publication and replacement use the same
staged transaction without requiring an old generated tree.

No run that generates zero traces exits `0`. On a fresh checkout the pinned
oracle or the external inputs are absent, and that is expected rather than
broken -- but it means the tracer's worklist will be short by three documents,
so it gets its own status instead of being folded into either success or
failure:

- `0`: every selected document was regenerated (and, for a full run, the
  contract was rewritten). The final line names how many.
- `1`: generation ran and failed -- an oracle run, a determinism comparison,
  or a fixture bootstrap did not hold.
- `2`: the command line is wrong.
- `3`: a prerequisite is absent, so nothing ran. Run
  `scripts/build-tex82-oracle.sh` and `python3 scripts/provision.py worktree .`,
  then rerun. Separated from `1` so a caller can tell "not set up yet" from
  "set up, and broken".

Identity is pinned in the committed
`tests/tex82-document-trace-manifest.txt`:

```text
document NAME ROOT-SOURCE FIXTURE-MANIFEST-SHA256 EVENTS FONT-SET-SHA256
```

The fixture manifest digest transitively pins every source, output, and event
byte (`CommittedFixture::load` verifies each file against it), the event count
pins the trace's scale, and the font-set digest -- a SHA-256 over one
`name sha256` line per staged TFM in bytewise name order -- pins the exact
metrics canonical replay registers. A present trace tree that disagrees with
any of these fails the run loudly rather than silently becoming the new
expectation. The same font-set digest is also the fixture manifest's
`distribution_sha256`, because the staged metric set _is_ this tier's
distribution.

Because `MainControl::resolve_font_resource` returns
`ExecError::MissingFont` immediately instead of suspending, the
tracer registers the whole staged TFM set through
`CommandHostCapabilities::register_font` before the first replay step rather
than through a lazy resource-host retry loop. Replay is bounded by
`(registered input bytes + expected event count) * 2 + 64` deliveries;
exceeding that bound is a contained worklist entry, not an aborted run,
because a document that expands far more commands than it has source bytes
must still terminate under a defect.

## First-Failure Locator

`crates/umber/examples/first_failure_locator.rs` is a standalone diagnostic
entry point for the `umber2-johp` command-core migration, separate from the
DVI-parity Cargo tests in [Testing Infrastructure](testing_infrastructure.md) and from the `umber2-johp.28` production
migration itself. Use it for the live end-to-end front, when the differential
tracer's fixture registry does not cover the failing input -- for example, it
depends on live document/font/hyphenation material outside
`tests/corpus/command`.

It stages `third_party/corpus/{plain,<source>}.tex`,
`third_party/hyphen/hyphen.tex`, and the plain-format CM/`manfnt` TFMs into an
in-memory `World` (reusing the same `parity_harness::CORPUS_TFMS`/`locate_tfm`
resolution as the parity harness), then drives them directly through
`umber::EngineSession`, so it exercises the ordinary command-core path:

```bash
cargo run --profile test -p umber --example first_failure_locator -- gentle
cargo run --profile test -p umber --example first_failure_locator -- story
```

Use `--profile test` (matching `cargo run-dev`'s alias) rather than the plain
`dev` profile: Gentle and Story are large documents, and an unoptimized
`opt-level = 0` debug build of the engine path can take
several minutes where the `test` profile's `opt-level = 1` finishes in
seconds.

It reports the first failure it hits: the live execution mode, the
`ExecError`/`SessionError` rendered with provenance-resolved TeX
source context (`ExecError::format_with_provenance`), or, for a Rust panic,
lets the default panic hook report the Rust-side `file:line` origin (rerun
with `RUST_BACKTRACE=1` for a full backtrace). As a first-failure locator (see
the Glossary in
[Canonical Divergence Working Contract](canonical_divergence_workflow.md#glossary)),
it can only show that
execution stopped, never that completed output is wrong. It intentionally
does not run under `cargo test`: the command core is mid-migration and this
locator is expected to fail on `gentle` until each earlier divergence in the
`umber2-johp` chain is fixed. `story` currently completes cleanly and is a
regression gate (see [Testing Infrastructure](testing_infrastructure.md#canonical-story-and-gentle-regression-gates)): a new `story`
failure is a regression to fix immediately, not the divergence under
investigation. See the current open successor issue under the `umber2-johp`
epic (`bd show umber2-johp` for its children) for the earliest tracked Gentle
divergence it reproduces -- that issue ID advances every time a divergence is
fixed, so it is not pinned here -- and `docs/tex_command_core.md` for the
canonical command-core architecture it exercises.
