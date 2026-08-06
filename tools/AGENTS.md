# Tools Guidance

`tools/refexec` is an opt-in host-side regeneration utility: it runs the machine reference TeX (`pdftex`, falling back to `tex`) in a fresh temporary directory, captures stdout/log/DVI outputs, and leaves repository inputs untouched. By default the tool locates `pdftex` or `tex` on `PATH`; set `UMBER_REF_TEX=/absolute/path/to/pdftex` to point fixture regeneration at a different reference binary, such as a specific TeX Live installation. Exact DVI normalization/comparison is owned by `test-support`; `refexec` re-exports and uses that shared contract for its CLI comparison paths.

`tools/fixturegen` is the sole host-side fixture publication owner used by `scripts/regen-fixtures.sh` and primary-checkout provisioning. Its `CasePlan`, `ArtifactSpec`, and `AtomicCaseTransaction` cover ordinary text/native updates, layout and PDF migration, externally staged cohorts, command-semantic batches, end-to-end reference DVI publication, and corpus acquisition. It is intentionally not a root workspace member; build it via `cargo build --manifest-path tools/fixturegen/Cargo.toml`. It may invoke `refexec`, `umber`, `pdftex`, `pdftoppm`, and `tftopl`, but cargo tests must not build or run it.
Candidate closed-directory validation and non-authoritative staging are shared from `test-support::closed_case`; only fixturegen may turn a validated candidate into repository authority.

`tex-command-stream`'s `command-semantic-schema` binary prints the structural JSON Schema generated from the V2 Rust manifest type; the committed corpus schema must match it exactly.

The `exec`, `typeset`, and `etex_exec` log generators run the reference in
INITEX mode (extended for `etex_exec`) and stage the seven printable catcode
assignments that `umber run` installs without loading a format. This matches
parameters and lexical conventions without inheriting Plain's assignments.
Their staged sources resolve repository-owned Computer Modern requests to the
owning worktree's absolute TFM paths, so regeneration never substitutes an
ambient TeX installation's font metrics.

`fixturegen --migrate-layout --plan` deterministically inventories the
single declarative registry of execution/output, lexical/session, and native
bibliography-invocation specifications in `layout_migration.rs`, reports each
case's file/byte census and domain-separated SHA-256, and performs no writes.
`--apply` stages and byte-verifies the entire requested plan before any
authority mutation. Its commit renames every old authority into a named
transaction backup and installs the staged cases; a failure reverses every
completed rename, reports every restoration failure, and retains recoverable
backups when restoration is incomplete. Transaction roots are atomically
allocated unique siblings and carry a strict schema/version/plan-digest
ownership marker. Commit occurs only after every installed directory is
byte-revalidated. Transaction removal after that point is garbage collection:
failure keeps the complete new authority, reports committed status and the
exact owned retained root, and a matching retry finishes cleanup. Unknown or
mismatched transaction roots are preserved and refused. A completed apply and
a successfully rolled-back apply are both safe to repeat. The reusable specification declares
case discovery, relative sources and destinations, roles, local metadata,
shared-input copies, and output mappings without assuming `.tex` or
`expected.<channel>` names. Family areas may be normalized nested paths, and a
shared authority may target either every discovered case or an explicit
nonempty subset; the latter keeps overlapping scenario groups declarative
without introducing duplicate flat authorities.

`fixturegen --migrate-pdf-layout --plan` inventories the fixed 15-case bounded
PDF cohort, including derived TrueType bytes and exact font, encoding, PK, and
included-PDF inputs. `--apply` stages and seals every complete candidate before
one reusable cohort Plan/Apply handoff; repeating apply validates the installed
closed cohort without consulting former shared authority.

`fixturegen --cohort-transaction --plan PLAN.json` and `--apply PLAN.json`
reuse that transaction engine for generators that already hold a complete
multi-case cohort. The versioned JSON plan names the Git checkout, each
repository-relative staged closed case, its unique repository-relative
destination, and every tracked authority to consume. Each staged directory is inventoried as closed local data; when it carries
`closed-case-v1` `case.inventory`, the declaration is checked exactly. Staged
output need not be tracked, while old authorities
are independently required to belong to the selected Git checkout. The command
canonically owns the schema, normalized destination, staged-case inventory, and
normalized authority paths for every case; case and authority ordering do not
change that ownership digest. Preflight rejects every ancestor or descendant overlap among staged cases,
destinations, authorities, and the transaction-root namespace before authority
inspection or mutation. A destination may name itself as its one consumed
authority for closed replacement. Commit
revalidates the exact inventory of the full cohort, including cases that were
already complete when the transaction began. Initial and retry-time
post-commit cleanup failures both report `committed=true` and the exact owned
retained transaction root, whether cleanup made zero or partial progress. The command
prints one `umber-fixture-cohort-result-v1` JSON object on success and exits
nonzero without a success object on validation, transaction, rollback, or
garbage-collection failure. The classic BibTeX regeneration path stages and
seals all nine complete closed candidate directories, then invokes this
interface once for the cohort.

Its `--classic-bibtex-differential` mode is called only by the `bibtex` branch
of `scripts/regen-fixtures.sh`. It generates a fixed, bounded seed corpus of
legal `.bst` programs, stages each case without host lookup, and compares
reference/Umber status, BBL, and BLG bytes. Failures are preserved under
`target/bst-differential/failures/` with their exact seed and inputs.

`refexec` also wraps `tftopl` for the font metric check owned by `tools/fixturegen`. When running that tier, it locates `tftopl` on `PATH`; set `UMBER_REF_TFTOPL=/absolute/path/to/tftopl` to point regeneration at a specific TeX installation.

`fixturegen --sync-corpus` is the external document acquisition mode run by `python3 scripts/provision.py worktree .` in the primary checkout. It reads the line-oriented `tests/corpus-manifest.txt`, preserves entry and locator ordering, fetches exact support inputs and runnable documents into gitignored `third_party/corpus/`, verifies SHA-256, and treats a complete cached hash match as a no-op. A changed corpus is installed as one closed atomic tree. Once setup is complete, conformance tests consume only local inputs and require no network access. Do not normalize line endings or commit fetched corpus files; licensing determinations live in the manifest notes.

`tools/texlive-wasm-publish` is a standalone release tool for browser TeX Live assets. It verifies every configured TEXMF root against a pinned tree digest, flattens lookup precedence deterministically, and writes an immutable manifest plus content-addressed objects. Build and test it explicitly with `cargo test --manifest-path tools/texlive-wasm-publish/Cargo.toml`; it must not join the root workspace or make ordinary tests scan a TeX Live installation.
Its manifest model and canonical serialization come from the workspace
`umber-distribution` crate; schema changes must keep the shared Rust/JavaScript
fixtures green.
Production snapshots use `python3 scripts/provision.py snapshot`, which scans the
full runtime-requestable TeX Live tree, derives bounded package hints from the
pinned `texlive.tlpdb`, and enforces inventory floors. The smaller
`build-wasm-latex-bundle.sh` remains a focused LaTeX seed/fixture builder and
must not be used for production publication.
The publisher's explicit `html` profile instead builds a new schema-4
distribution from selected format closures, runtime TeX/TFM objects, and an
exact curated WOFF2/mapping/license catalog. It does not mutate or filter the
schema-3 production snapshot in place.

`tools/parity-harness` is the shared Rust library and opt-in compatibility CLI for end-to-end DVI conformance. Oracle-presence-conditional Story, Gentle, TRIP, and e-TRIP tests use its default library for final artifact comparison against gitignored, locally generated `tests/corpus/e2e` DVI files, without compiling live reference execution. Its fixture path stages manifest inputs and calls an in-process Umber runner supplied by the Cargo test; it never launches the Umber binary. The `reference-tools` feature can execute reference TeX and return manifest-verified DVI bytes, but canonical publication belongs to `fixturegen --reference-dvi`. Comparison uses `test-support` to normalize only DVI preamble comments, requires byte-identical final DVI, and writes automatic bundles under `target/conformance-triage/` or the CLI-selected triage directory.

`tools/parity-harness/src/trip_triage.rs` owns the compact TRIP-specific v1
artifact. It compares canonical `tex-oracle` event streams before transcript,
log, and preamble-normalized DVI channels, writes no copied outputs, and keeps
the deterministic report bounded. For e-TeX sparse-register shorthand
delivery, it projects the reference engine's allocator-owned array-node
address only when Umber supplies the portable register-class and
`print_sa_num` identity; an absent or integer Umber operand remains a semantic
divergence.
Its macro-address projection follows explicit group lifetime: a local meaning
mutation temporarily invalidates the outer macro proof, and TeX82 §282's
`unsave` at the matching `\\endgroup` restores it. Global meaning mutations
update every saved projection scope because §282 retains them rather than
restoring the saved value.
The reference e-TeX instrumentation's
`protected_delivery_suppression` splice is projected out rather than copied
into the engine: e-TeX [53a] returns the protected command directly from
`get_token`, and [37.785]/[37.791] use that result as alignment lookahead. The
raw command and its later backup/template replay remain exact gating events.

`tools/profile-analyzer` is the read-only Samply/Firefox processed-profile CLI.
It reconstructs columnar sample stacks, consumes Samply presymbolication
sidecars including inline frames, and reports self/inclusive, subtree, and
runtime-caller attribution for persistent engine profiles.

`tools/tex-command-stream` is the offline, test-only canonical command-stream
comparison runner. It replays TeX82 command fixture inputs through the
instrumented command boundary, delegates owned observer translation to the
lower `tex-observe` crate, and reports a ranked worklist of up to
`--max-divergences` ordered divergences (stream mismatches and contained
replay failures alike). It never invokes a reference engine or joins the
production engine dependency graph.

`event-stream-diff EXPECTED.jsonl ACTUAL.jsonl` is the read-only exhaustive
counterpart for already-captured canonical streams such as guarded TRIP
artifacts. It applies the same keyed alignment and exact root-site grouping,
with no divergence budget. It intentionally compares the portable events as
captured; a format-loaded caller must still distinguish allocator-only macro
operands using the phase projection recorded by the TRIP triage report.

`tex-command-stream::policy` is the sole semantic-comparison owner. Its named
ordinary policy returns bounded aligned divergences and ordered/root-site
accounting together; both the repository runner and `event-stream-diff`
consume that result. Its strict TRIP policy parses and validates canonical
streams, applies phase-aware macro and reference-instrumentation projection,
and returns the first positional divergence plus complete raw/projected
accounting from the same walk. Parity renders that result and owns no event
projection or event-counting pass.

The tool's one Cargo integration binary is `tests/it.rs`; focused external-boundary suites are submodules under `tests/it/`. `tests/it/command_semantic.rs` owns the generic declarative semantic-minifixture runner. It discovers each fixture's singleton versioned `manifest.json` under `tests/corpus/command-semantic/<domain>/<fixture>/`; every fixture directory is a closed unit containing that manifest, its declared TeX source, and each applicable `expected.<channel>` file, with no domain manifest or shared expected-output tree. The runner validates catalogue ownership and exact provenance, drives tiny fixture bytes through instrumented `MainControl`, and enforces short exact pass or strict-xfail projections without adding Rust case registries or integration binaries. Manifests may select filtered canonical observation families and supply bounded in-memory terminal lines or named inputs, so pausing, read, and input-open cases remain hermetic. They may also select committed command, mode, final-box, and prepared-page artifact boundaries for focused execution evidence. While `umber2-alfh.11` owns the terminal-EOF divergence, the two corpus-wide exact-comparison tests run manually with `cargo test -q -p tex-command-stream --test it command_semantic -- --ignored`; schema, inventory, route, and bounded behavioral checks remain routine.

Comparison is a two-tier keyed sequence alignment, not an index-parallel
scan: `src/compare.rs` splits each event into an identity key and a payload,
reports a payload-only difference once without desynchronizing, and repairs a
key mismatch with a bounded wavefront search (`--realign-window`,
`--realign-confirm`) followed by a structural anchor fallback that rejoins at
the least-total-skip shared anchor inside `--anchor-scan`, stopping a
fixture's comparison outright rather than guessing when neither confirms.
Each entry names its repair and the cascade it suppressed. See "Stream
alignment" in `docs/testing_infrastructure.md`.

`src/group.rs` then collapses exact recurrences of one root site into a single
entry, and `src/report.rs` renders the worklist. Grouping is presentation
only: the comparison, the entry order, and the divergence count are identical
with and without it, `--ungrouped` prints the one-entry-per-divergence
worklist, and the header prints both totals with labels saying which is
comparable against historical figures. Two divergences group only when they
are equal after erasing every source position and nothing else, through an
exhaustive match over the event schema, so a new schema variant fails to
compile rather than silently changing what merges. The report also prints the
bounds it applied: a fixture whose `--max-divergences` budget was reached, and
a document trace that was never generated. Either bound also makes the run
`PARTIAL` (exit `2`), separate from `CLEAN` (`0`), `DIVERGED` (`1`), and a run
that could not be performed (`3`), so a caller reading only the exit status
cannot take an uncompared fixture for a converged one; the report is printed
even when clean and closes with a matching `VERDICT:` line. See "Grouped
worklist and run accounting" in `docs/testing_infrastructure.md`.

The runner owns no canonical vocabulary of its own. Every name it puts in an
event -- catcodes, command names, glue orders, scanner statuses, token
spellings -- comes from `tex_command::canonical_names`, and the command name
comes from the producer's record rather than being re-derived from the
spelling. Keeping a second table here is how `umber2-johp.141` found a
transport-side catcode table that had drifted from tex.web's §207 names in
seven places while silently masking an engine divergence; see "Canonical
Observation Vocabulary" in `crates/tex-command/AGENTS.md`.

It replays two registries: the committed, always-present fixtures under
`tests/corpus/command/tex82`, then the full-document traces (plain bootstrap,
Story, Gentle) under `tests/corpus/command/tex82-documents`. The document tier
is generated on demand by `scripts/build-tex82-document-traces.sh` and
gitignored -- those traces run 17-156 MB each -- with its identity pinned in
`tests/tex82-document-trace-manifest.txt`; an ungenerated document is skipped
with a stderr notice rather than failing. Document replay registers the staged
TFM set through `CommandHostCapabilities::register_font` before its first step,
because canonical font resolution fails rather than suspends. See
"Differential Tracer" in `docs/testing_infrastructure.md`.

The Cargo integration test calls the committed-only runner. That entry point
enforces fixed, name-independent source-count, combined-source-byte, and event-
count ceilings before replay, so adding a document-scale fixture to the
committed registry fails with an actionable diagnostic. Only the explicit CLI
runner loads `tex82-documents`; do not route automated tests through it or
raise the ceilings to accommodate a full document.

`python3 scripts/provision.py worktree .` acquires the external hyphenation and
Computer Modern font inputs from the pinned TeX Live 2026 runtime and fetches
and verifies the pinned official Knuth TeX82 TRIP and e-TeX V2 e-TRIP
materials. Cargo integration tests execute the
two-phase format workflow directly in Rust and reuse the pinned `trip.tfm` for
e-TRIP. Fixture regeneration independently runs the two-phase reference
workload with pdfTeX.
