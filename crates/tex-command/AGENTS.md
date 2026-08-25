# tex-command Guidance

Read the repository-level `AGENTS.md` and `docs/tex_command_core.md` before
editing this crate.

## Crate Role

`tex-command` is the sole target owner of canonical source tokenization, input
levels, raw command delivery, expansion, macro calls, conditions, scanners,
alignment delivery, and static profile dispatch. It depends on `tex-state` and
must never depend on `tex-exec`; `tex-exec` consumes its completed
unexpandable commands.

`docs/expansion_memory_lifetimes.md` is the current implementation map and
retention audit for command-state generations, execution scratch, macro and
scanner nesting, input ownership, and suspension. Update it whenever this
crate changes an owner or exact reclamation point.

Host capabilities are borrow-scoped through `CommandHostContext` and must
never enter snapshots, formats, durable summaries, or owned command state.
Private state-machine modules must not be widened for compatibility with
retired input or expansion APIs.

Line acquisition is **not** a host capability. tex.web §31's `input_ln` is
`tex_state::CommandContext::input_ln`, alongside `\ifeof`'s §501
`read_stream_at_eof`, because a terminal line and a `\read` stream are
`Universe` state rather than a host acquisition keyed by request, and because
`tests/it/boundaries.rs` forbids `CommandHostContext` from every file on the
line path. It is the one line source for both of tex.web's callers -- §363's
`firm_up_the_line` and §483-§486's `\read` -- and §71's `prompt_input(#)`
prints its own prompt inside the acquisition, so the command core has no
print channel of its own outside the borrowed
`tex_state::CommandContext::begin_diagnostic` diagnostic channel
`\tracingifs` uses with the executor's operation-local detached-effect
collector (see `src/conditionals.rs`).

## File Map

- `Cargo.toml`: dependency-light crate manifest and boundary-test support.
- `src/lib.rs`: intentionally small public facade and private module tree.
- `src/attempt.rs` and `src/attempt/tests.rs`: transitional scanner/operation
  scratch, including scanner-owned sinks, promotion, and suspension scope
  capabilities. Macro invocation storage does not use this arena.
- `src/execution_scratch.rs`: current-generation reusable execution scratch.
  Macro argument words use stable linked fixed-size chunks from one coarse
  pool, fixed nine-range slot metadata, private branded cursors, and intrusive
  O(1) chunk-chain recycling; activations never own a heap buffer or arena
  scope.
- `src/host.rs`: borrow-scoped, nonserializable host-capability boundary.
- `src/profile.rs` and `src/profile/tests.rs`: public semantic character values,
  immutable command/character profiles, the distinct canonical compiled-engine
  semantics that survive loading an older format, capabilities, stable
  fingerprints, and focused value/identity tests.
- `src/state.rs`: copy-on-write aggregate command roots, persistent command
  state, cross-processor executor-owned replay-completion fences, and current-
  generation execution scratch. Resource continuations retain the exclusive
  current-generation lease, its same scratch lanes, typed ids, and integer
  resume cursors; resumption re-borrows dense state and cancellation drops the
  current candidate wholesale. These are process-local command state, never
  format or summary payload.
  Also owns `\tracingnesting`'s `record_source_open_depths`/
  `source_open_depths`, the `grp_stack`/`if_stack` recording e-TeX 2.6
  [23.328] compares at a source level's `end_file_reading`.
- `src/command.rs`: public opaque, ephemeral current-command representation;
  its cloneable value crosses only executor preflight/retry seams and never a
  durable snapshot or format boundary.
- `src/processor/expand.rs`: canonical expanded delivery, including the
  same-borrow preflight settlement that reports and discards undefined
  commands before returning the following command to the executor. The
  ordinary driver owns one live
  current command and lends it through macro and ranked primitive expansion;
  it moves that value into continuation state only after a typed immutable-host
  suspension, and a resumed primitive retains §367's already-emitted trace
  instead of printing the command twice. The same rule covers the typed `\expandafter` operand and
  `\csname` accumulator frames.
- `src/processor/mod.rs`: processor construction plus the opaque delivery
  cursor moved across an executor-owned typed resource continuation; it
  restores observation ordering but owns no command/input semantics.
  Production borrows use `CommandProcessor::borrowed`, which takes the
  session-owned fuel, observer, and operation-local diagnostic-effects
  collector directly and constructs no temporary owned ledger.
- `src/error.rs`: command error and resource-need representation plus the
  shared dimension-scanner recovery diagnostic vocabulary consumed by legacy
  and canonical scanner paths.
- `src/fuel.rs`: checked finite command-work limits, monotonic scalar command
  work counters, the constructor-free borrow-only `CommandFuel` capability
  used by leaf operations, and the top-level `CommandFuelLedger` session owner
  shared by every canonical processor episode. Fuel and work counters are
  operational evidence outside semantic state; rollback never refunds them.
- `src/fatal.rs` and `src/fatal/tests.rs`: TeX82 §93 `fatal_error`,
  §94 `overflow`, and §95 `confusion` as one shared irrecoverable-error
  value, its canonical observation record, and focused label tests. Every
  present and future `succumb` site raises this type; §81 `jump_out` is
  modelled by the executor latching it as the session's terminal state.
- `src/input/source.rs`, `src/input/source/tests.rs`: public host-neutral
  one-shot source-registration inputs and errors plus private immutable backing,
  source cursors, retained World modification metadata for typed file
  enquiries, and focused registration tests. It also owns
  `SourceNameClass`, TeX82 §303's partition of a source level's `name` into
  the terminal (`name=0`), input stream `name-1` (`1..=17`), e-TeX's
  `\scantokens` pseudo-file (`name=18` or `19`), and a text file (`name>19`).
  That is a third, independent classification: `InputReason` is
  a strict model of §307's `token_type` codes, which a source level does not
  have at all, and `RegisteredSourceKind` names how Umber acquired the bytes
  rather than which channel TeX reads them as. An open with no explicit class
  is §537 `start_input`'s file, because §537 is how TeX reaches every `\input`
  and the job's root file alike; only §331's terminal and §483's `\read`
  streams need `open_registered_source_as` (`umber2-johp.245`). It also owns
  `SourceRegistration::with_name` (§537's `a_make_name_string`) and
  `FileFramingEvent`, the queued `Open`/`Close` record of when a named `File`
  level or traced `\scantokens` pseudo-file opened or exhausted. Named `File`
  levels are command-framed by default; the explicit
  `SourceFramingPolicy::ExternallyOwned` exception preserves an authored root's
  `File` identity while suppressing duplicate events when its surrounding host
  wrapper already owns that transcript frame. The queue exists because the input stack is reached
  from places that hold no `Universe`; `CommandState::render_file_framing_events`
  prints it as tex.web's `(name`/`)` bracketing through
  `tex_state::file_framing`, and the processor drains it the instant a source
  retires, because §362 prints its `)` ahead of the `check_outer_validity`
  diagnostic on the next line. Whatever the core could not render itself the
  engine drains once per step.
- `src/input/lines.rs`, `src/input/lines/tests.rs`: exact physical-line
  splitting, TeX line normalization, byte/scalar cursor and range accounting,
  and focused line-contract tests.
- `src/input/tokenizer.rs`, `src/input/tokenizer/tests.rs`: canonical
  token-at-a-time exact-byte and separately identified UnicodeExtended M/N/S
  tokenization, semantic control-sequence spelling, profile-specific
  superscript notation, invalid-character recovery steps, byte/scalar ranges,
  and focused conformance tests.
- `src/input/levels.rs`, `src/input/levels/tests.rs`: canonical fixed-width
  source/token cursors over chunk-owned packed traced words and shared durable
  token-list cursors, cold backup source
  coordinates, explicit stored/transient/backed-up TeX82 cell ownership,
  orthogonal delivery/retirement classifications, and compact
  admitted-macro replacement and argument-span coordinates. A source level's `open_depths` field is
  `\tracingnesting`'s own record; see `src/tracing_nesting.rs`.
- `src/input/stack.rs`, `src/input/stack/tests.rs`: exact input retirement,
  centralized packed-chunk admission, retained v-template lifecycle,
  macro-activation cleanup, `param_start` parameter replay ownership, and
  trace-independence tests.
- `src/processor/`: public borrow-only processor facade with specialized raw
  and expanded delivery loops, expansion, scanner-status, and alignment
  orchestration. The loops share the typed policy boundary but do not test a
  raw-versus-expanded mode on every token.
  The facade also resumes an executor-retained settled delivery and settles a
  raw preflight command without backing it up or delivering it twice.
  `status.rs` owns the one processor-level scanner episode mechanism for
  typed status entry, observation visibility, recovery re-entry, and complete
  prior-state restoration; scanner families do not open-code that lifecycle.
- `src/processor/tests.rs`: tracked command-root publication and fail-closed
  unsupported-continuation coverage.
- `src/processor/alignment.rs`, `src/processor/alignment/tests.rs`: canonical
  alignment-delivery state and focused stack, brace-depth, template, and omit
  lifecycle tests.
- `src/processor/expand.rs`, `src/processor/expand/tests.rs`, and
  `src/processor/fixtures/`: ordinary expanded-command delivery, expandable
  primitives, converted-token construction, focused private unit tests, and
  bounded source microfixtures.
- `src/scanners/`: private typed scanner family. `hyphenation.rs` owns TeX82
  §934/§960's `\hyphenation`/`\patterns` scans, which are `get_x_token`
  classification loops rather than `scan_toks` collections and so must never
  enter the `absorbing` scanner status or track a brace depth.
  `expression.rs` and `expression/tests.rs` own e-TeX 2.6's four typed
  expression scanners and two glue-level conversions, including their
  explicit parenthesis stack, arithmetic recovery, glue-order rules,
  observations, and checkpoint-retry tests.
  `structured.rs`'s `scan_accent_base` is deliberately a one-command step
  rather than a loop: TeX82 §1123 runs §1270's `do_assignments` between the
  accent code and §1124's base character, and executing an assignment is the
  executor's, not the scanner's. It replays only §1124's own `else
  back_input`, inside the delivery episode that fetched the command; a
  prefixed command is handed back still delivered, because §1270 executes it
  in place and a backup level here would deliver it twice.
  `scalar.rs` owns `back_input_unless_spacer`, TeX82's
  `if cur_cmd<>spacer then back_input`. §443's `⟨Scan an optional space⟩`,
  §444's `⟨Scan a numeric constant⟩`, and §452's `⟨Scan decimal fraction⟩`
  all end a numeric scan with that one rule, so every numeric scan routes its
  terminator through it rather than choosing per call site whether to absorb
  a space. Expandable `\number` and `\romannumeral` scans retain their leading
  sign/provenance, radix-tail accumulator, or completed §442 character code
  awaiting its expanded optional-space probe when delivery suspends on an
  immutable host request, so retry resumes the exact TeX82 §§440--445 token probe. The terminator test is
  on the command, so it is the category code and never
  the character: §207 makes `spacer` the command a category-10 character
  carries, and §349 is what normalizes such a character's `cur_chr` to a
  space inside §341's `get_next`. The same reusable ABA-tagged scalar lane owns
  optional-equals, fixed-inline keyword prefix, integer, dimension, glue,
  filename, internal-value, expression, and font-selector continuation state.
  Raw resource-capable scanners remain private; public retained calls return a
  move-only completed/suspended/failed result, and each expansion,
  conditional, alignment, structured scanner, or executor operation moves the
  suspended child into its exact typed phase. Success, resuspension, abort, and
  fallible parent-frame storage all close or reinstall that chain
  deepest-first. Do not add a root mailbox, caller-order result tape,
  destination inference/search, or command redispatch fallback.
  `filename/tests.rs` owns focused expanded filename scanning, termination,
  replay, and registered-source retry tests.
  `structured.rs`'s `scan_math_field_episode` is TeX82 §1151's `scan_math`,
  a classification and not an absorption: every scalar case ends holding one
  math code, so the field pushes no input level, backs up no token, and never
  redelivers the command that selected it, and `othercases` is the whole
  remaining vocabulary reaching §1153's `back_input; scan_left_brace`. A
  frozen-spelling replay level here delivers each field twice
  (`umber2-johp.265`).
  `restricted.rs` owns TeX82 §433-§437's five restricted integer classes as a
  single mechanism: every bounded scan in the crate selects a
  `RestrictedIntegerClass` instead of open-coding a range test, and the
  recover-to-zero belongs to the scan, never to the command consuming it.
- `src/primitives/`: private static primitive dispatch. The integrated
  catalogue spans `primitive_metadata.rs`'s exhaustive enum rows,
  `parameters.rs`'s parameter cells/defaults, and `generated.rs`'s exceptional
  meanings (`nullfont`, frozen `endwrite`, page quantities, and extension
  internals). `generated.rs` is the sole projection seam for stable operands,
  exact profile name sets, installation slices, observation identities,
  prefix policy, and deterministic documentation tables. `registry.rs`
  consumes those views for fresh INITEX and format restoration; it contains
  only store-local installation mechanics. `catalogue.rs` defines the
  behavior-free descriptor vocabulary and exhaustive validation. Execution
  dispatch remains handwritten.
  `prefixed.rs` owns TeX82 §209's `max_non_prefixed_command` partition -- the
  single test §1211's `prefixed_command`, §1270's `do_assignments`, and the
  `\global` prefix all make. It is narrower than "this command assigns
  something": `\begingroup`, `\endgroup`, `\aftergroup`, `\afterassignment`,
  `\openin`/`\closein`, and every `extension` primitive are excluded, so a
  caller that starts from a broader notion and subtracts them by hand is
  re-deriving this predicate one exception at a time.
- `src/macro_call.rs`, `src/macro_call/tests.rs`: private canonical scalar
  macro matcher, destination-directed construction into execution-scratch
  chunks, stable sealed slot descriptors, sequential parameter replay cursors,
  exact tail/nested retirement, and focused tests.
- `src/conditionals.rs`: private independent condition-stack machine; also
  renders e-TeX 2.6's `\tracingifs` `{...}` trace lines at conditional entry
  and at each `\or`/`\else`/`\fi` delimiter resolution, printed directly
  through `tex_state::CommandContext::begin_diagnostic` into the enclosing
  operation's detached-effect collector because tex.web's `show_cur_cmd_chr`
  fires from inside `conditional`/`pass_text` itself rather than through the
  executor.
- `src/tracing_nesting.rs`: renders e-TeX 2.6's `\tracingnesting`
  `file_warning` -- "Warning: end of file when ... is incomplete" for every
  group and conditional still open at a source level's natural EOF, compared
  against the depth `state.rs`'s `record_source_open_depths` recorded when
  that level opened. Called from `processor/next.rs`'s `retire_and_restart`,
  the one choke point every input-level retirement passes through. Prints
  through the ambient selector (`CommandContext::printer`), not
  `begin_diagnostic`'s `\tracingonline` redirect: unlike
  `\tracingassigns`/`\tracinggroups`/`\tracingifs`, `file_warning` is not
  `stat`-gated in `etex.ch`. The same module owns `if_warning` and the
  ordinary/semi-simple `group_warning` close path. `\scantokens` pseudo-files
  record the same opening depths as ordinary inputs. Remaining specialized
  group-close sites are tracked by `umber2-aqx9`.
- `src/scan_toks.rs`, `src/scan_toks/tests.rs`: private canonical token-list
  scanner and focused parameter, collection, expansion, scanner-status, and
  recovery tests. A scanner owns no arena or scope. Temporary collection uses
  the scanner word/builder lanes, while a surviving token list is built and
  sealed directly in its final current-generation destination. Nested macros
  use separate macro frame/argument lanes, so push/pop never interleaves their
  scratch with scanner output. A suspended scan carries branded frame indices
  under the same exclusive current-generation lease. Its semantic
  `ScanToksMode` constructors are parsed once
  into a typed internal grammar, opener, expansion, warning owner,
  observation purpose, and status-visibility configuration. It also owns
  TeX82 §482's `read_toks`, which is deliberately
  _not_ a `scan_toks` mode: it collects whole lines rather than a
  brace-balanced group, holds `align_state` at `1000000` for its whole
  duration, and continues across a brace imbalance instead of ending at a
  closing brace. §486 does not balance a runaway `\read` by inventing
  braces; its whole recovery is `align_state:=1000000; limit:=0` plus the
  error, so the stored list keeps exactly the tokens the file supplied.
- `src/observation/`: private aggregate read observation. `mod.rs` owns the
  record union, engine-owned source identity and alignment nesting, and the
  exhaustive `Meaning`-level command classification;
  `primitive_identity.rs` and `variable_identity.rs` own the exhaustive
  primitive and eqtb-addressed variable identities beneath it. None of the
  three may reintroduce a catch-all: an unclassified meaning must be a build
  failure, never a plausible generic identity in a trace.
  `variable_identity.rs` also owns `parameter_mutation_key`, the single
  supported way to name a mutated named parameter. Umber's dense parameter
  bank slots are not tex.web's parameter codes, so an observation that
  formats a raw `IntParam`/`DimenParam`/`GlueParam`/`TokParam` slot reports
  one parameter's assignment under another's name (umber2-johp.134).
  `canonical_names.rs` generalizes that rule to the whole vocabulary: it is
  the only place any observation name is spelled, and it is re-exported as
  `tex_command::canonical_names` so producers in other crates and the
  differential tracer share one table rather than each keeping their own.
  See "Canonical observation vocabulary" below.
  Command identity is selected through the immutable `CommandProfile`:
  `variable_identity.rs` owns the TeX82/e-TeX 2.6/pdfTeX 1.40.29 eqtb and
  `last_item` layouts, while `primitive_identity.rs` owns profile-dependent
  conversion selectors. Never classify an observed command through a
  profile-free dialect approximation.
- `src/snapshot.rs` and `src/snapshot/tests.rs`: generation-generic command
  snapshots and named summaries backed by the command-root timeline and
  containing one coarse generation owner plus fixed scalar journal, arena,
  stack, source-anchor, and profile coordinates; capture never clones a live
  command graph, validation never mutates the runtime, and capture requires
  quiescent execution scratch. Restore installs roots before resetting scratch
  lanes and truncating unpublished storage suffixes.
- `src/continuation.rs` and `src/continuation/`: handle-free command-summary
  and suspended-execution recipes, dense DTO-local indices, recursive schema
  validation and budgets, cold detachment construction, destination-stamped
  staging, atomic publication, and focused rejection/retry tests. The schema
  contains no runtime identity, owner, storage coordinate, or borrow.
- `tests/`: external dependency, visibility, and capability-boundary tests.
  Character/input integration coverage binds the exact shared-domain tokenizer
  to the pinned TeX82 fixture and compile-fail gates profile immutability.

## Canonical Observation Vocabulary

Every string an observation payload carries for a _concept_ -- a category
code, a character command, a scanner status, a glue order, a token's catcode
or spelling, a meaning's command name -- is spelled once, in
`src/observation/canonical_names.rs`, and nowhere else. Producers in
`tex-command` and `tex-exec` and the differential tracer all call it.
Scanner results use `ObservationValue`; producers must retain integer, scaled,
glue, name, and token-list domains instead of rendering values for a detached
consumer to parse. Mutations pair a typed `MutationTarget` with typed key and
value fields; effects pair `ObservationEffectKind` with separate channel and
value fields. Neither family may encode structure with prefixes, separators,
or numeric text for `tex-observe` to reverse.

Rules, all of them load-bearing (umber2-johp.141):

- **tex.web is the authority, never Umber's enum spellings.** §207 fixes the
  category codes and the command codes that share their numeric values; §135
  fixes the glue orders; §305 fixes `scanner_status`; §289/§365 fix how a
  token is represented. Umber's Rust variants are `Superscript`, `EndLine`,
  `Parameter`, `Ignored`, `Active`, `Invalid`; tex.web's names are `sup_mark`,
  `car_ret`, `mac_param`, `ignore`, `active_char`, `invalid_char`.
- **The catcode table and the character-command table are different tables.**
  They agree only at codes 1..8 and 10..12. Catcode 0 is `escape` but command
  0 is `relax`; catcode 9 is `ignore` but command 9 is `endv`; catcode 13 is
  `active_char` but command 13 is `par_end`; catcode 14 is `comment` but
  command 14 is `stop`; catcode 15 is `invalid_char` but command 15 is
  `delim_num`. Keeping one table and reusing it for both is a naming defect
  that will look correct on most inputs.
- **A Rust `Debug` rendering must never reach an observation payload,** and
  never round-trip through one either: `Debug` spells Umber's variant names
  and the oracle spells tex.web's, so any agreement is accidental. A
  `format!("{x:?}")` in a record field, or a transport that prefix-matches a
  record field against a Rust variant name, is a bug on its own terms
  regardless of whether a fixture currently notices.
- **Each family gets one total function with a single entry point.** No
  silent catch-all: an unnameable value either has an explicit,
  tex.web-cited arm or gets a deliberate name no engine installs
  (`undecodable_meaning`, `uncommandable_character`) so that it looks like the
  internal defect it is. Adding a `Catcode`, `Order`, or `ScannerStatus`
  variant must fail to compile until it is placed deliberately.
- **Never re-derive a name a producer already computed.** A transport that
  recomputes a command name from the spelling instead of carrying the
  producer's is a second, divergent table; it will silently mask engine
  divergences that the producer's name would have exposed, which is exactly
  what a spelling-derived command name did until umber2-johp.141 deleted it.
