# Reachability-owned token, macro, and glue values

Status: architecture contract for Beads issue `umber2-3v8z.3` and its
children.

This document defines the immutable-value ownership boundary for token lists,
macro definitions, and glue specifications. It complements the live-state
identity contract in [Core Engine State](core_state.md) and the private
revision lifecycle in [Private revision allocation domains](patch_allocation_domains.md).

## Invariant

Persistent value memory is owned by a current semantic root, live restoration
authority, an explicit loaded-format base, or a detached published value. A
store lookup structure is never an owner:

```text
typed strong root -> immutable exact-content object
                         ^
                         |
                  weak or evictable index
```

Each object contains its complete immutable semantic payload and its cached
versioned semantic identity. Hashes select candidates only. Reuse requires an
exact structural comparison, including symbol semantic atoms for token lists
and recursively referenced exact objects for macro bodies. A collision may
cost lookup work but cannot alias two values.

Runtime slots, generations, capacities, allocation-domain coordinates, weak
index membership, and observation operands are physical metadata. They do not
enter live identity, format identity, memo identity, or exact equality.

Dynamic weak-slot cleanup is incremental. Each lookup or allocation advances a
fixed-size cursor through the slot table and a lookup cleans only its queried
candidate bucket. A dead value therefore costs bounded metadata work on the
ordinary hot path even when a long live prefix precedes it. Generation-safe
slot reuse remains authoritative; a hard admission limit may perform one exact
complete sweep before degrading or reporting exhaustion. This sweep owns no
semantic reachability and retains no history. Operation rollback resets the
cursor to the discarded suffix's captured physical extent, so repeated retries
reuse their warmed high-water slots without scanning an unrelated frozen or
live prefix.

There is no copying compactor, reachability sweep, accepted-generation
registry, or after-the-fact graph traversal. Every strong edge is installed or
removed at the typed mutation boundary that already knows both the owner and
the referenced value.

## Audited owners and restoration edges

The pre-migration stores own dense successful-history vectors. Aggregate
snapshots retain watermarks into those vectors, so a successful redefinition
cannot be reclaimed even after its Env cell, save-stack entry, page, node, and
checkpoint disappear. The following typed owners replace that accidental
authority.

| Value              | Current strong owners                                                                                                          | Restoration or transfer edges                                                                                  |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------- |
| Token list         | Env token registers and parameters; macro bodies; command stored input and active macro calls; nodes and page marks; PDF state | Env undo entries; command summaries; Universe snapshots; generation forks; format bases; memo imports; patches |
| Macro definition   | Env meaning cells and active macro calls                                                                                       | Env undo entries; command summaries; Universe snapshots; generation forks; format bases; patches               |
| Glue specification | Env glue registers and parameters; compact and owned nodes; page `last_glue` and insertion state                               | Env undo entries; Universe and page snapshots; generation forks; format bases; memo imports; patches           |

The token-list row expands into the following concrete owner matrix. This is
the durable pre-migration audit for `umber2-3v8z.3.1.2`; a field listed here
may keep its compact `TokenListId`, but it must hold an owning reference beside
that coordinate before the legacy successful-history arena is retired.

| Ownership stratum                     | Concrete live owners                                                                                                                                                                                                                | Restoration, detachment, or transfer edge                                                                                                                                                                                                         |
| ------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Store and loaded base                 | The canonical empty value and every validated schema-11 frozen token-list row. Dynamic weak slots and candidate-index buckets are explicitly not owners.                                                                            | Frozen decode validates complete token content and symbol atoms before publishing an explicit immutable base. Future append uses the same exact-content pool without extending base ownership.                                                    |
| Environment current cells             | Dense and sparse token registers, token-list parameters, and token-valued immutable format-base cells in `Env`.                                                                                                                     | Local/global assignment changes the compact word and strong sidecar atomically. Raw format-overlay installation does the same after validating the frozen-base reference.                                                                         |
| Environment restoration               | Token-valued `Journal::Undo` old values, including equal local assignments and entries refiled by later global assignment.                                                                                                          | Group compaction, `unsave`, rollback, and journal truncation move or drop the saved/current owner at the same record transition that handles the raw word. `EnvSnapshot` cloning shares both current and saved roots.                             |
| Macro children                        | `MacroStore` definitions' parameter and replacement token lists. The later macro-body migration may change the containing representation, but these two child edges are token roots now.                                            | Macro interning, format loading, rollback, generation fork, command-continuation materialization, and later macro reclamation preserve or release both child owners together.                                                                     |
| State input summaries                 | `TracedTokenList`, `InputFrameSummary::TokenList`, and `MacroReplaySite` while it can survive independently of the delivering frame.                                                                                                | Summary freeze/restore and checkpoint comparison share the owner; root-revision rebind changes only source frames. Transient inline token buffers remain separately owned semantic data.                                                          |
| Command live state                    | `SourceLevel::every_eof`, `TokenPayload::Stored`, alignment cell u/v templates, active macro-body replacement delivery, queued named-token-list observations, and completed scanner/request values that cross the processor borrow. | Command operation snapshots and `CommandSummary` clones share roots. Input retirement, template completion, request application, failure rollback, and `NeedResource` retry drop or restore the exact level/request root.                         |
| Detached command continuation         | `OwnedCommandContinuation` is a handle-free logical DTO. It owns source descriptors and bytes, symbol spellings, token/macro/origin/list/frame recipes, and portable command scalars; it owns no `CommandSummary` or runtime root.  | Materialization validates the complete recipe graph, installs it in a staged destination fork, and swaps that fork only after a complete destination-local `CommandSummary` exists.                                                               |
| Execution mode                        | `AlignState` columns' u/v templates and any operation-local execution value retained after command delivery.                                                                                                                        | Mode snapshots, rollback-journal entries, checkpoint summaries, and aggregate operation retry clone or swap the corresponding owner with the compact field.                                                                                       |
| Owned and compact nodes               | `Node::Mark`; deferred write/special/PDF-literal whatsits; PDF action, destination, thread, and annotation fields within owned nodes; compact mark and whatsit sidecars.                                                            | Builder freeze validates and captures each owner. Arena suffix rollback drops it, survivor promotion shares it, and node-format or memo detachment converts it to semantic DTO data.                                                              |
| Page and split state                  | Current/split/top/first/bottom marks, mark-class maps, contribution/current-page nodes, insertion records, fire-up state, and page snapshots.                                                                                       | Copy-on-write page mutation, split/fire-up replacement, page rollback, shipout commit, memo import, and checkpoint restore swap whole persistent roots or one typed field without reconstructing ownership from ids.                              |
| World effects                         | Unexpanded `EffectRecord::DeferredWrite` entries still retained in the live or detached effect ledger.                                                                                                                              | Effect rollback, prefix commit/splice, resource retry, accepted-output transfer, and final materialization carry or release the owner with the exact effect record.                                                                               |
| PDF state                             | Token parameters; page/form records; raw objects; document fragments; annotations and links; action identifiers/targets/specs; destinations, threads, outlines, and open-link state.                                                | Each copy-on-write PDF collection and `PdfStateSnapshot` structurally shares owners. Rollback, page-suffix transfer, format capture/load, and final detached PDF publication preserve exact content. Cached semantic fingerprints are not owners. |
| Aggregate checkpoints and generations | `Universe` snapshots plus command and mode summaries own the closure of all rows above. Retained incremental checkpoints and generation forks share those immutable owners.                                                         | Operation rollback restores typed roots before patch-domain truncation. Checkpoint restore and generation fork retarget physical coordinates while retaining exact objects; selected-root acceptance transfers only explicit rooted objects.      |
| Formats and memos                     | `StoreFormat`, frozen-core DTOs, PDF format DTOs, node DTO token tables, and `DetachedMemoValue` own handle-free semantic content while detached.                                                                                   | Capture enumerates the already-known typed closure. Load/import validates and interns exact content, then installs the returned owner immediately; malformed or rejected DTOs publish no runtime root.                                            |
| Private patch domain                  | A private revision domain owns each newly allocated token object in addition to any typed private destination.                                                                                                                      | Failed operation and `NeedResource` restore destinations then truncate the operation suffix; candidate rejection drops the domain; acceptance validates a deterministic typed root list and transfers only distinct selected payloads.            |

### Implemented Env and macro token-root stratum

`umber2-3v8z.3.1.2.2` installs the environment and macro-child rows above. The
compact token words remain unchanged, but ownership is now explicit and moves
at the same boundary as each word:

| Transition                     | Current-cell owner                                                                   | Undo or child owner                                                                                           | Release boundary                                                                                       |
| ------------------------------ | ------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Dense or sparse token write    | `Env` replaces the cell's `TokenListRef` after the typed bank write succeeds.        | A newly appended token-valued undo receives cloned old and new roots, including equal local assignments.      | Replacing the current cell drops its former root; journal truncation drops both recorded roots.        |
| Token-parameter write          | Null owns no value; present empty and nonempty values carry their exact strong root. | The optional encoding and strong roots are attached to the same `TokParam` undo position.                     | Replacement, group exit, or rollback drops the displaced optional owner at that typed boundary.        |
| Later global supersession      | The surviving global value becomes the current owner.                                | Group compaction moves the global redo owner and refiles the first outer owner into the enclosing undo slice. | Truncating the compacted group releases superseded local old/new roots without scanning store slots.   |
| Group exit or rollback         | Restoration installs the undo record's old root before publishing the restored word. | Refiled global records retain their exact old/new roots; ordinary removed records are dropped with the slice. | The journal suffix and its owners are truncated only after the destination root has been restored.     |
| Raw or frozen format install   | Each token-valued immutable `FormatBaseCell` owns its validated decoded token value. | The mutable overlay receives the same owner; later raw-global undo records receive exact old/new roots.       | Overlay restoration exposes the still-owned immutable base; replacing a base is not a job operation.   |
| Macro construction and loading | The macro row retains its compact child ids.                                         | Parallel parameter and replacement `TokenListRef` columns own both immutable children.                        | Macro rollback/truncation drops both child columns with the definition; a fork clones the strong refs. |
| Generation fork                | Cloned Env current and format-base owners share immutable payloads.                  | Cloned journal records and macro child columns share payloads while their stores mint fork-local coordinates. | Dropping either generation releases only that generation's roots.                                      |

Snapshots remain O(1) journal positions: they do not clone the entire Env.
Their rollback authority is the current Env plus the still-live journal
prefix, whose token-valued records now own every value they may restore.
Canonical identity continues to resolve exact token content and ignores root
counts, slot generations, and journal representation. No compaction or graph
scan was added.

### Command, mode, and checkpoint token-root audit

The bounded pre-change audit for `umber2-3v8z.3.1.2.3` identifies the exact
compact-coordinate fields in this stratum. Read-only function arguments and
short-lived rendering locals are not owners.

| Owner                        | Pre-migration compact field                                                                                                                            | Required strong edge                                                                                                                                                                                         | Typed release or restoration boundary                                                                                                                   |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| State input value            | `TracedTokenList::token_list`                                                                                                                          | The traced pair owns a `TokenListRef`; origin ownership remains independent.                                                                                                                                 | Dropping the traced value releases the token edge. Snapshot, request, and replay clones share it.                                                       |
| State input summary          | `InputFrameSummary::TokenList::token_list`                                                                                                             | The frozen summary owns a `TokenListRef`.                                                                                                                                                                    | Summary replacement, rollback, checkpoint release, or generation-fork remap drops or replaces it atomically with the frame.                             |
| Macro delivery provenance    | `MacroReplaySite::token_list`                                                                                                                          | A replay site clones the active macro-body token owner while the site can outlive its delivering frame borrow.                                                                                               | Dropping the completed delivery releases the clone; snapshot and retry preserve it with the delivery.                                                   |
| Stored command input         | `TokenPayload::Stored::tokens`                                                                                                                         | Every live stored cursor owns its exact token value.                                                                                                                                                         | Input retirement drops the cursor owner; command snapshot rollback restores the cloned cursor root.                                                     |
| Source EOF replay            | `SourceLevel::every_eof` through `TracedTokenList`                                                                                                     | The source level owns the once-only list until it is moved into a stored cursor.                                                                                                                             | Natural EOF moves the owner into the replay cursor; source retirement or rollback drops or restores it.                                                 |
| Alignment delivery           | `AlignmentCellTemplates` u/v lists and completed preamble/cell values                                                                                  | Each template owns a `TokenListRef` through its traced value.                                                                                                                                                | Cell/template completion, suspension/resume, command rollback, and preamble handoff move or clone the exact roots.                                      |
| Command publication queue    | `CommandState::named_token_list_pushes` token coordinate                                                                                               | Each queued observation owns the list until publication.                                                                                                                                                     | Draining the queue drops the observation owner after rendering; command rollback restores the queue roots.                                              |
| Completed scanner or request | `InternalValue::Tokens` and every `TracedTokenList`-bearing result/request                                                                             | The completed owned value carries a `TokenListRef` across the processor borrow.                                                                                                                              | Applying, rejecting, or retrying the enclosing operation consumes or drops the result; the aggregate step snapshot restores prior roots.                |
| Active macro delivery        | `push_macro_activation` replacement coordinate                                                                                                         | The replacement cursor receives a clone of the macro definition's exact replacement owner before becoming visible.                                                                                           | Macro-body retirement drops the cursor root independently of later redefinition; the macro activation retains its definition owner separately.          |
| Execution alignment mode     | `AlignColumn::{u_template,v_template}`                                                                                                                 | Each mode column owns both template values.                                                                                                                                                                  | Mode-journal capture/rollback, mode summary capture/restore, alignment completion, and checkpoint release clone, move, or drop the columns as a unit.   |
| Detached continuation        | Dense DTO-local indices into canonical source, symbol, token, macro, origin-list, and expansion-frame recipes plus portable cursor and command scalars | Detachment borrows the live summary only while copying its logical closure. Materialization validates every index, cycle, range, source descriptor, activation, and cursor before staging destination roots. | Rejection drops the staged fork and leaves the destination unchanged; success swaps one complete destination generation and returns its owning summary. |
| Aggregate checkpoint         | `EngineCheckpoint::command` and `EngineCheckpoint::modes` nested token coordinates                                                                     | The command and mode summaries structurally own every token value they name.                                                                                                                                 | Retained restore clones roots before the `Universe` branch switch; checkpoint eviction or generation replacement drops the complete closure.            |

The conversion keeps `TokenListId` as the compact coordinate returned by an
owning reference. Equality, hashing, tracing, effects, and output continue to
use semantic token content. No graph compaction, store walk, or checkpoint-time
root discovery is introduced.

### Implemented command, mode, and checkpoint token-root stratum

`umber2-3v8z.3.1.2.3` installs the audited edges above without changing the
portable representation. Runtime values retain `TokenListRef`; observation,
format, detached-continuation, and checkpoint identity continue to project
canonical token content rather than owner counts or process-local pointers.

| Transition                           | Strong-root behavior                                                                                                                                                                                                | Exactness control                                                                                                                                                                                                                                  |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Command snapshot and operation retry | Cloning command state shares stored cursors, source every-eof values, scanner results, alignment deliveries, and queued observations. Rollback replaces the failed state with that owned clone.                     | A stored cursor may retire and its source Universe may disappear before rollback; the restored cursor still delivers identical tokens. Existing scalar/expression/structured retry tests continue to compare exact command state and observations. |
| Active macro delivery                | Activation clones the macro row's replacement child into the live body cursor before publishing the level. Later assignment of the control sequence changes neither that cursor nor its origins.                    | A focused redefinition test activates one replacement, redefines the macro, and observes the original replacement from the already-active cursor.                                                                                                  |
| Alignment command-to-mode transfer   | Completed u/v templates move from command-owned traced values into `AlignColumn` roots. Mode summary and inverse journal entries share those roots.                                                                 | Destructive alignment-state removal followed by rollback restores the exact u/v content after all live destination owners are dropped.                                                                                                             |
| Durable command continuation         | Detachment copies source-relative cursor recipes, token semantics, control-sequence spellings, macro bodies/provenance, origin lists, and expansion-frame ancestry into a DTO with no live summary or runtime root. | Materialization into a Universe with deliberately different coordinates produces destination-local token, macro, origin, list, frame, and symbol identities while preserving exact restart and diagnostic content.                                 |
| Aggregate checkpoint                 | The command and mode summaries nested in the checkpoint own their reachable token values independently of live command/mode destinations. Restore clones those summaries before switching Universe state.           | A retained checkpoint restores both a retired command cursor and removed alignment templates, then delivers identical command and mode content.                                                                                                    |
| Typed release                        | Input retirement, scanner/request consumption, alignment completion, journal truncation, summary/checkpoint eviction, and continuation replacement drop their exact owner fields.                                   | The weak immutable pool remains non-authoritative. Traced values move directly into typed node, page, or effect owners without a successful-history compatibility root.                                                                            |

The ordinary operation lifecycle still restores destinations before private
patch truncation on `NeedResource` or scanner failure. No new root walk,
compaction phase, or store-wide checkpoint discovery was added.

### Assignment observation token roots

e-TeX 2.6 [19.277--279] observes an eqtb token-list pre-image before
`eq_destroy` can release it, performs the assignment, and then observes the
post-image. Umber's execution boundary combines that sequence in
`AssignmentCommitter`, so token-register and token-parameter commits clone
typed pre/post `TokenListRef` values before mutating `Env`. The assignment
trace renderer borrows those exact values directly and never resolves a bare
operation-local `TokenListId` after the write.

This owner is necessary even when an ordinary local assignment happens to
retain the pre-image through an undo sidecar: a global write, redundant-write
shortcut, or later journal disposition must not determine whether the
operation-local observation stays readable. The roots disappear when the
commit-and-trace call returns. They add no history owner, index authority,
compaction pass, or graph scan.

### Node, page, PDF, and effect token-root audit

The bounded pre-change audit for `umber2-3v8z.3.1.2.4` identifies the
remaining runtime owners in this stratum. Read-only projections, semantic-hash
callbacks, token expanders, and handle-free detached bytes are not owners.

| Owner                                            | Pre-migration compact field                                                                                                                          | Required strong edge                                                                                                                                                           | Typed release, restoration, or transfer boundary                                                                                                                                                                                   |
| ------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Owned node                                       | `Node::Mark::tokens`; deferred write, special, and PDF-literal whatsit tokens; PDF thread attributes and token-valued action identifiers             | Each owned node value owns every token value named by its fields before it can enter a page queue, mode list, box, memo graph, or freeze builder.                              | Dropping or replacing the owned node releases its fields; moving it into compact storage transfers the same roots without an unrooted interval.                                                                                    |
| Compact node                                     | Mark rows and token-bearing whatsit/PDF sidecars in `NodeStorage`                                                                                    | Every compact sidecar holds the exact `TokenListRef` beside the logical token coordinate exposed by `NodeRef`.                                                                 | Arena suffix rollback truncates the root-bearing sidecars; compact copying and survivor promotion clone the roots; final survivor release drops them.                                                                              |
| Node detachment and materialization              | Token-table indices in node format and memo DTOs                                                                                                     | Detachment copies canonical token data from the node's owning references; materialization interns the data and installs destination-generation roots before publishing a node. | Failed validation publishes no node or token root. Successful format/memo import transfers only the newly materialized owned graph.                                                                                                |
| Page scalar marks                                | Top, first, bottom, split-first, and split-bottom `TokenListId` plus presence bits                                                                   | Each present page mark is one optional strong token root; present empty is distinct from absent.                                                                               | `set_mark`, clear, page reset, split, fire-up, and snapshot rollback replace or drop the exact option at the typed page mutation.                                                                                                  |
| Mark classes                                     | `MarkClassState::marks` plus its presence bitmap in the persistent class map                                                                         | Each present slot in every nonzero class owns its token value, including the canonical empty value.                                                                            | Copy-on-write class mutation shares untouched roots and replaces one slot; removing the final slot drops the class row and all of its roots.                                                                                       |
| Page node collections and snapshots              | Contribution, current-page, discard, insertion, best-break, and fire-up roots containing owned or compact nodes                                      | The persistent page roots structurally own the token-bearing nodes above.                                                                                                      | Split/fire-up replacement, page rollback, checkpoint eviction, memo import, and generation fork share, swap, or drop the complete persistent closure.                                                                              |
| Deferred World effect                            | `EffectRecord::DeferredWrite::tokens` in live effects, snapshot roots, page prefixes, and detached `EffectJournal` values                            | The effect record owns the unexpanded token value through one strong reference.                                                                                                | World rollback truncates the effect suffix; prefix commit/splice, journal slice/concat, prepared-suffix transfer, and accepted revision publication clone or move the complete record; final expansion or record drop releases it. |
| PDF token parameter                              | `PdfTokenParameter::tokens` in page parameters, frozen PK mode, raw-object data/stream attributes, document fragments, and form attributes/resources | The parameter wrapper owns its token value and its cached semantic fingerprint remains non-authoritative.                                                                      | Typed parameter replacement, raw-object initialization, document append, page/form commit, rollback, format capture/load, and page-suffix transfer clone, move, or drop the wrapper as one value.                                  |
| PDF action                                       | User-action text; file, structure, page-view, and destination identifier token fields in action specs and action records                             | Every token-bearing action component owns its exact value. Catalog, link, outline, thread, and node users structurally own the complete action.                                | Scanner completion hands an owned action to the executor; reservation/application, node freeze, snapshot rollback, collection replacement, and final detachment preserve or release the full action closure.                       |
| PDF annotation and link                          | Annotation entries; link attributes and action; open-link copies                                                                                     | Each initialized annotation and each logical or open link owns the token fields it names.                                                                                      | Initialization publishes all roots atomically; link start/end and copy-on-write collection rollback move or drop the record without reconstructing ownership from ids.                                                             |
| PDF outline, destination, and thread             | Outline attributes/title/action plus token-valued destination/thread identifiers retained by records or nodes                                        | Each record owns all token values reachable through its typed fields.                                                                                                          | Collection append, duplicate lookup, snapshot restore, node lowering, and final PDF publication borrow or clone the record roots; collection or node release drops them.                                                           |
| PDF page, form, object, and document collections | `PdfPageRecord`, `PdfFormRecord`, `PdfRawObjectRecord`, `PdfDocumentFragments`, catalog open action, and their copy-on-write roots                   | Each collection element structurally owns its token wrappers. `PdfStateSnapshot` shares the collection roots and the frozen PK-mode parameter.                                 | PDF rollback swaps the complete roots; page-suffix take/restore moves page owners; format load validates and roots every token before publishing state; clearing the final record releases its fields.                             |
| PDF detachment and suffix publication            | Handle-free format DTO token bytes, detached finalization values, committed pages, and accepted page suffixes                                        | Live detachment borrows strong owners and emits canonical bytes. Any live page suffix remains owning until its detached consumer no longer needs runtime token expansion.      | Successful detachment leaves no runtime coordinate in the DTO. Page-suffix transfer moves owning records exactly once; rejection or rollback drops the transferred suffix.                                                         |
| Generation and aggregate rollback                | `Universe` page, PDF, node/survivor, and `World` snapshot roots                                                                                      | A fork shares immutable token payloads while retaining generation-local coordinates through the owning wrappers already embedded above.                                        | Local retry restores typed roots before patch-domain truncation; checkpoint/fork replacement drops the losing generation's closure at the aggregate boundary.                                                                      |

The implementation may keep `TokenListId` as an accessor result and compact
coordinate, but no runtime aggregate listed above may store an authoritative
bare id. Present-empty values remain rooted where their owner distinguishes
them from absence. Format, memo, and final PDF DTOs remain handle-free and do
not become token-store authorities.

Read-only accessors, hashing callbacks, token printers, shipout expanders, and
format encoders are borrows, not roots. A scanner result is a root only for the
interval in which it is an owned completed request awaiting application.

The audit includes these less obvious edges:

- an equal local assignment may still create an Env undo owner;
- a later global assignment may retain, refile, or supersede a local undo;
- group exit and rollback can exchange the current and saved owners even when
  their raw words compare equal;
- a command input level can outlive redefinition of the macro whose replacement
  text it is delivering;
- page marks, insertion split glue, leaders, whatsits, PDF actions, PDF object
  data, document fragments, annotations, and outlines own values without an
  Env cell;
- a checkpoint owns its Env journal, input summary, page/PDF roots, command
  summary, mode/node roots, and loaded-format base as one closure;
- generation fork retargets runtime coordinates while sharing immutable
  payloads; and
- detached format and memo DTOs own semantic data rather than live runtime
  handles.

Transient borrowed reads are not roots. A builder owns unfinished content;
freezing transfers one strong reference to its typed destination or returns an
owned value that must be consumed by a destination. No API may return a bare
new runtime coordinate whose payload is kept alive only by the store.

### Implemented node, page, PDF, and effect token-root stratum

`umber2-3v8z.3.1.2.4` installs the audited edges above. Owned nodes and compact
mark/whatsit sidecars now carry `TokenListRef`; page scalar and mark-class
slots carry optional roots; deferred writes own their unexpanded payloads; and
PDF parameters, actions, annotations, links, outlines, raw objects, document
fragments, pages, forms, catalog state, snapshots, and suffixes structurally
own every token-bearing field. Public and compact accessors still project
`TokenListId`, while hashing and detached DTOs remain owner-independent.

The transition roots once at scanner or `Universe` admission and moves or
clones the typed wrapper thereafter. Node freeze and survivor promotion, page
copy-on-write mutation, World journal splice, PDF collection rollback, page
suffix transfer, and generation fork therefore preserve ownership without a
graph walk. Focused controls drop the final typed destination and observe the
strong count return to the test observer alone; node rollback additionally
proves that the weak token-store slot is no longer live.

Node memo detachment reads canonical content through the compact owners and
materialization publishes destination-local roots only after validation.
Final PDF detachment borrows token coordinates from the still-owning records
and emits handle-free bytes. This stratum adds no graph compaction and changes
no portable format.

### Completed format, memo, and acceptance closure

`umber2-3v8z.3.1.2.5` removes the temporary successful-history token owner.
The token store now retains only the immutable frozen base, weak reusable
dynamic slots, bounded weak candidate buckets, test-only detached fixture
owners, and private-domain handles. A bare `TokenListId` can resolve only
while a typed owner keeps its exact object live.

Format capture projects the current weak slot extent, represents dead physical
holes as empty only until the existing dense reachable-closure pass removes
them, and serializes exactly the Env, macro-child, and node roots already
known to the typed format boundary. Loading publishes the validated frozen
base before installing Env, macro, and node owners; later append and
redefinition use ordinary weak-slot interning. Memo token import returns an
owning `TokenListRef`; macro and node materialization install their child roots
before publishing the containing value. Detached DTOs remain handle-free.

Private acceptance walks the deterministic token allocation order, not the
state graph. Each newly allocated private token value receives a weak
acceptance lease whose strong half exists only in the initially returned typed
owner and its descendants. Resolving a domain-only payload cannot mint a
lease, and an unrelated payload `Arc` clone therefore cannot select an
unrooted allocation. Operation rollback removes exact handle/lease suffixes;
rejection drops the domain; acceptance transfers only allocations whose typed
lease is live, then clears all private metadata.

The final production `TokenListId` audit classifies every remaining bare use
as one of these non-owning cases:

- an API parameter, accessor result, observation operand, printer/expander
  input, semantic-hash projection, or compact Env/node word whose containing
  typed structure owns the parallel `TokenListRef`;
- a handle-free format, memo, continuation, or PDF detachment coordinate that
  is validated and immediately materialized into a typed owner; or
- `MacroMeaning`'s parameter/replacement coordinate, whose current
  `MacroStore` row owns parallel strong child references and whose containing
  macro-body representation is deliberately reserved for `.3.2`.

No production aggregate retains a bare authoritative token id. No graph scan,
compactor, accepted-generation registry, or successful-history owner remains.
Focused controls cover weak-index eviction and collisions, stale coordinates,
format/source/future append and redefinition, memo import, operation rollback,
resource retry, rejection, selected acceptance, a live unrelated payload clone,
10,000 bounded-live redefinitions, and exact all-roots-live growth.

### Macro-body and binding owner audit

`umber2-3v8z.3.2` separates the exact immutable body from each definition
occurrence that binds or invokes it. A body structurally owns its flags,
preparsed parameter structure, parameter token-list root, and replacement
token-list root. A definition reference owns that body plus optional
diagnostic metadata; its `MacroDefinitionId` remains only the compact
timeline-local coordinate used by `Meaning`. Equivalent definition
occurrences may therefore retain distinct TeX observation operands and
provenance while sharing one exact body object.

| Ownership stratum             | Concrete strong owner                                                                                     | Restoration, release, or transfer edge                                                                                                                                                                |
| ----------------------------- | --------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Weak store lookup             | None. Body candidate buckets and definition slots contain weak references only.                           | Lookup/allocation advances bounded weak-slot reclamation; a candidate hash collision still performs exact flags and child-content comparison.                                                         |
| Loaded-format base            | Every validated frozen definition row has one explicit immutable definition root.                         | Decode validates flags and token child indices before publishing the base. Dynamic future definitions use ordinary weak slots and do not extend base ownership.                                       |
| Environment current binding   | Each macro-valued meaning cell owns its exact definition reference beside the packed meaning word.        | Local/global assignment replaces word and owner atomically. Redefinition drops the displaced current root only after any required undo root is installed.                                             |
| Environment restoration       | Each macro-valued undo old/new word owns the matching definition reference.                               | Equal local writes, global supersession/refiling, group exit, journal rollback, and truncation move or drop word and root together.                                                                   |
| Active command invocation     | Each `MacroActivation` owns its definition reference independently of the replacement cursor.             | Activation publication clones the delivered definition root before the body level is visible. Input retirement, retry rollback, continuation detachment/materialization, and summary release drop it. |
| Frozen primitive registry     | A driver-selected macro sentinel owns its definition beside the primitive meaning and frozen-token index. | Profile reconstruction and generation fork clone this operational owner; replacing or dropping the registry releases it independently of Env bindings.                                                |
| Aggregate checkpoint and fork | Env current/undo roots and command-summary activation roots form the checkpoint's macro closure.          | Snapshot rollback restores those typed roots before private allocation rollback. A generation fork shares immutable bodies while retaining fork-local coordinates and complete binding roots.         |
| Formats and memos             | Frozen formats own definition roots; detached format, memo, and continuation DTOs own semantic bytes.     | Capture serializes only the explicit reachable binding closure. Load/materialization validates, interns the exact body, creates a destination definition root, and publishes it atomically.           |
| Diagnostic provenance         | An optional sidecar belongs to one live definition reference, never to the semantic body or index.        | Missing or stale origins degrade to unknown. Provenance records may retain a non-owning physical definition operand, but cannot upgrade it or keep the definition/body alive.                         |
| Private patch domain          | The private domain owns each new body and definition allocation in addition to live typed references.     | Failed operation truncates its exact allocation suffix; rejection drops all private allocations; acceptance enumerates typed allocation leases and transfers only live roots and structural children. |

Read-only `macro_definition`, state hashing, observation rendering, format
encoding, and provenance resolution are borrows. `MacroInvocationOrigin`'s
definition coordinate is diagnostic metadata and intentionally is not a root.
There is no compatibility successful-definition history: the next TeX
observation operand is fixed-size rollback state, while obsolete operands and
sidecars disappear with their final definition reference.

### Glue-spec owner audit

The bounded pre-code audit for `umber2-3v8z.3.3` classifies every live
`GlueId` edge below. A compact id may remain in packed Env words, node words,
observations, and handle-free DTOs, but it is not ownership. Every runtime
aggregate that can outlive the borrow from which it obtained an id owns the
parallel exact `GlueSpecRef`.

| Ownership stratum                      | Concrete strong owner                                                                                                                                                                           | Restoration, release, or transfer edge                                                                                                                                                                                                                                                                  |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Weak store lookup and zero value       | The canonical zero glue and validated loaded-format rows are explicit immutable roots. Dynamic slots and candidate buckets are weak, bounded, and non-authoritative.                            | Each lookup/allocation advances bounded reclamation; reuse mints a fresh generation. Candidate-key collisions still compare all five glue fields exactly.                                                                                                                                               |
| Environment current cells              | Dense and sparse skip and muskip registers, glue parameters, and glue-valued immutable format-base cells own one exact root beside each packed word.                                            | Local and global assignment replace the word and root atomically. Raw format-overlay installation validates and installs the matching frozen root before publishing the word.                                                                                                                           |
| Environment restoration                | Every glue-valued undo record owns its old and new roots, including equal local writes and records refiled by later global writes.                                                              | Group compaction, `unsave`, aggregate rollback, and journal truncation move or drop roots at the same transition as their words. Restoration installs the saved owner before releasing the displaced current owner.                                                                                     |
| Owned and compact nodes                | Glue, leader, insertion split-top-skip, and PDF snap nodes own each glue value they name; compact rows retain parallel roots in `NodeStorage`.                                                  | Builder freeze captures roots before publication. Arena suffix rollback drops them, compact copying and survivor promotion clone them, and final list release drops them. Node-format and memo detachment emit semantic glue data rather than runtime handles.                                          |
| Page state                             | `PageBuilderState::last_glue`, contribution/current-page/discard/insertion/best-break/fire-up node roots, and page snapshots own their scalar or node-contained glue closure.                   | Page append and reset replace `last_glue` with the node's root; split, fire-up, rollback, checkpoint eviction, and generation replacement share, swap, or drop the persistent page roots without a store scan.                                                                                          |
| Execution mode and unfinished builders | Paragraph left/right/fill skips, alignment default and boundary tabskips, unfinished mode lists, split/page-output requests, and operation-local builders own their glue values until transfer. | Command-to-mode handoff, mode-journal rollback, checkpoint summary capture/restore, list freeze, split completion, and failed-operation rollback move, clone, or drop the typed owners directly. Pure typesetting snapshots may copy `GlueSpec` semantic values and are not runtime-handle authorities. |
| Aggregate checkpoints and forks        | Env current/undo roots, mode summaries, page roots, and compact/survivor node roots form the checkpoint's glue closure.                                                                         | Retained checkpoint restoration clones these typed roots before switching aggregate state. Generation fork shares immutable payloads while minting or resolving generation-local coordinates; losing checkpoints or generations drop their complete closure.                                            |
| Formats and memos                      | Loaded formats own every validated glue row as an explicit frozen base. Detached format and memo DTOs own handle-free five-field glue data.                                                     | Capture enumerates only typed reachable Env and node roots. Load/import validates data, interns it, installs destination-local owners, and publishes the containing cell or node atomically. Future append uses ordinary weak slots and does not extend the frozen base.                                |
| Private patch domain                   | A private domain owns every newly allocated glue object in addition to typed private destinations and their weak acceptance leases.                                                             | Failure and `NeedResource` restore destinations before truncating the exact allocation suffix. Rejection drops the domain. Acceptance follows allocation order and transfers only allocations whose typed lease remains live, then clears private metadata.                                             |
| Non-owning projections                 | Scanner/expression source identities, assignment-trace old/new operands, semantic hashing, printers, packers, compact views, and DTO table indices borrow or project ids only.                  | These values never upgrade a dead slot and cannot retain content. A completed builder or scanner value that must survive application instead carries semantic `GlueSpec` data until a typed runtime destination interns and owns it.                                                                    |

This audit includes source and loaded-format construction, later append,
equal-local/global-supersession restoration, page and mode rollback,
node-survivor transfer, memo materialization, resource retry, candidate
rejection, selected acceptance, and generation fork. It deliberately adds no
successful-history owner, compatibility root, compactor, graph scan, or
checkpoint-time discovery pass.

### Implemented glue-spec closure

`umber2-3v8z.3.3` installs the audited edges above. `GlueStore` now retains
only the explicit zero and loaded-format roots, weak generation-safe dynamic
slots, a bounded weak candidate index, and private-domain allocation
metadata. Interning hashes all five glue fields and then compares exact
content, so width, stretch, stretch order, shrink, and shrink order all
participate in identity even when candidate keys collide. `GlueSpecRef`
carries the immutable value and timeline coordinate; `GlueHandle` lets an API
borrow that coordinate without consuming and prematurely releasing a sole
owner.

Env current cells and glue-valued undo records keep aligned strong sidecars.
Compact ordinary node words keep a one-for-one optional root column, while
leader, insertion, PDF snap, page, paragraph, alignment, line-breaking, and
operation-local structures carry typed roots directly. Assignment, group
exit, aggregate rollback, generation fork, node freeze, survivor transfer,
page reset, and mode restoration therefore clone, move, or drop owners at the
existing state transition; none discovers ownership by scanning the store.

Format capture extends the existing detached reachable-closure remap to glue
references in Env and node DTOs. A loaded format publishes all validated glue
rows as an immutable base, while subsequent additions return to weak dynamic
slots. Memo DTOs remain handle-free and import returns a destination-local
owner before publishing a containing node or result. Private glue allocations
join the aggregate token/macro acceptance order with typed weak leases;
operation rollback truncates their exact metadata suffix, rejection retains
nothing, and acceptance transfers only allocations selected by live typed
destinations.

Focused controls cover exact collision discrimination including stretch and
shrink orders, the permanent zero root, final-owner release and generation
reuse, source and loaded-format behavior, future weak append, Env
current/undo/group restoration, page and checkpoint rollback, memo import,
private retry/rejection/selected acceptance, 10,000 bounded-live
redefinitions, and exact all-roots-live object and byte growth. The migration
adds no successful-history owner, compatibility root, compaction pass, or
runtime graph scan.

## Object and slot representation

Each family has a private immutable payload behind shared ownership. An owning
reference pairs that payload with a timeline-local opaque coordinate used by
compact Env and node encodings. Related Universe forks share the payload but
mint or resolve coordinates through their own slot tables.

The dynamic slot table contains weak payload references and generation-safe
coordinate metadata. Dead slots are reusable without moving a live payload.
Reuse mints a fresh generation so a stale coordinate cannot alias later
content. Dead suffixes and reusable-slot metadata are trimmed or recycled at
ordinary allocation boundaries; capacity must plateau with the live and
configured cache high-water size.

The content index is either weak or explicitly bounded and evictable. Buckets
may disappear at any time. Lookup performs these steps:

1. compute the versioned semantic candidate key;
2. upgrade live weak candidates;
3. compare exact content, recursively where required;
4. reuse an exactly equal object or allocate a new immutable object; and
5. install only non-owning acceleration entries.

The canonical empty token list and zero glue may use immortal built-in values.
A loaded format owns an immutable frozen base explicitly. Later values use the
same exact lookup and reusable dynamic slots; the frozen base is not copied
into a successful-history overlay.

## Owner mutation contract

Env current cells and undo records store strong value references in parallel
with their compact semantic words. The write barrier changes both as one
operation. Journal coalescing, global supersession, group compaction, group
exit, rollback, and raw format-overlay installation must move or clone the
corresponding strong owner at the same point that they move or clone the raw
word. A receipt without the ownership disposition is insufficient.

Immutable nodes structurally own every token-list and glue value named by
their compact rows or sidecars. Node freeze validates and captures those
references before publication. Arena rollback drops the exact allocated
suffix and its references; survivor promotion shares the immutable references
with the promoted payload. This contract does not otherwise implement the node
promotion work tracked by `umber2-3v8z.6`.

Page, PDF, input, command, and mode structures store owning references in their
persistent copy-on-write roots. Replacing one field releases exactly that
field's old reference. Snapshot cloning shares these roots. Restoration swaps
the retained root; it does not reconstruct ownership by walking raw handles.

Macro body identity is distinct from both binding and provenance:

- the immutable body owns flags, parameter structure, parameter tokens, and
  replacement tokens;
- an Env meaning is the current binding of a control sequence to that body;
  Env undo records own older bindings; and
- definition and token-origin provenance is optional diagnostic metadata. It
  is excluded from semantic identity and cannot keep a semantic object alive
  merely through a lookup side table.

## Private revision lifecycle

Creating a new immutable value during private execution allocates its payload
in the revision's `PatchAllocationDomain` under the active aggregate operation
mark. The typed destination may also hold a strong reference inside the private
Universe. The domain remains the revision-level owner and records the exact
logical charge.

Ordinary failure and `NeedResource` rollback first restore typed destination
roots, then truncate the failed operation suffix. Earlier successful private
objects remain once. Rejecting the candidate or prepared transaction drops
both its roots and the complete domain.

Acceptance obtains deterministic typed roots from the owners that already
know them. The domain validates those roots and transfers only their distinct
objects. Structural children are owned by the selected objects themselves.
Unselected objects disappear with the domain. The accepted Universe retains
no domain, and acceptance performs no store walk or graph discovery.

## Identity, formats, and detached values

Canonical live-state hashing reads semantic identities through owning roots.
It never hashes physical coordinates or asks an index whether a value is live.
Equal reachable values therefore hash equally across dead allocation histories
and slot reuse; distinct exact content remains distinct even under a candidate
hash collision.

Format serialization assigns dense DTO indices from the explicit serialized
root closure and emits exact content once per DTO table. Format loading
validates complete content and child indices before publishing the frozen base.
Future append, redefinition, group restoration, and checkpoint rollback use
the ordinary object path above. A format must not serialize dead dynamic slots
or weak-index membership.

Memo and command-continuation detachment remain semantic-data operations. On
materialization they intern exact content and immediately install the returned
owning references in the destination roots. Their detached maps are not live
store authorities and do not justify retaining unrelated runtime values.

## Verification obligations

Each migrated family must have focused controls proving:

- deliberate candidate-hash collisions still compare exact content;
- every owner class remains readable while it is the sole root;
- removing the final root makes the weak payload and reusable slot collectible;
- equal local assignments, global supersession, open-group undo, group exit,
  rollback, and retained checkpoints preserve exact owners;
- source and loaded-format construction, future append, memo import, and
  generation fork preserve exact content;
- failure and `NeedResource` drop only the failed operation suffix, rejection
  drops all private work, and acceptance retains only explicit roots; and
- thousands of distinct redefinitions with bounded live groups/checkpoints
  plateau in live objects, slot metadata, and index capacity independently of
  command count.

An all-roots-live negative control must grow by the exact number and logical
bytes of intentionally retained values. This distinguishes genuine reclamation
from a test that merely stopped allocating.

## Migration order

`umber2-3v8z.3` is split into clean representation closures:

1. `umber2-3v8z.3.1` introduces the shared weak-slot substrate and migrates
   token lists across every audited owner;
2. `umber2-3v8z.3.2` builds macro bodies and bindings on reachability-owned
   token lists; and
3. `umber2-3v8z.3.3` migrates glue values using the same substrate.

The parent closes only after all three children and their combined plateau
controls pass.
