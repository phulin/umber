# Box prefix grammar

Status: implemented redesign for `umber2-johp.772`; validation is in progress.

## Semantic boundary

TeX82 §645 scans the optional packing clause, saves the packing values, calls
`new_save_level(c)`, and then calls §403's `scan_left_brace`. §1083 enters the
new mode and schedules `every_hbox` or `every_vbox` after that scan returns.
§1167 uses the same prefix for `vcenter`. These are distinct semantic moments:
packing expansion belongs to the enclosing group, mandatory-brace expansion
belongs to the new group, and both run in the enclosing mode.

The former construction scanner consumed the brace before the executor opened
the group. It returned a construction record containing a second box-kind enum
that the executor immediately converted back into its actual kind. Both are
removed.

The independent oracle control is:

```tex
\setbox0=\hbox\relax\csname scopedprefix\endcsname{}
```

After this box closes, `scopedprefix` must be undefined. The leading relax
prevents either packing-keyword probe from expanding the csname. Moving that
csname inside the body must also keep it local. Removing the relax makes the
csname expand during the packing probe, so its resulting relax definition must
remain in the enclosing group. The three cases prevent moving group entry
before packing or leaving it after brace scanning.

## Actual owners

The executor selects its final `ReplayBoxKind` and `GroupKind` directly from the
terminal primitive and destination. The adjusted-hbox distinction remains at
that destination selection. The command processor receives the actual group
kind, scans the prefix, opens and traces the canonical group, and returns only
the actual packing value. The executor subsequently enters the mode, installs
its final active-box owner, and schedules the actual everybox input.

Group entry uses the existing command input line, state save stack, and
diagnostic effects. It creates no group facade, deferred group request, cached
group depth, or executor-owned copy of the TeX stack. The existing entry helper's
work moves into the command owner and is shared by its real callers.

`ScannedBoxConstruction`, `ScannedBoxKind`, their exports and conversion
adapters, `PackingScalarPhase`, and the construction-only opening wrapper are
removed.
Packing remains an actual operand, rather than a generic request or progress
record. Ordinary closing and final node ownership remain with their current
semantic consumers.

## One consuming grammar

Use the sole expanded reader to consume leading spaces and classify the first
substantive packing word once. Keyword spelling follows §407: only a literal
character can match, comparison ignores ASCII case and character category,
and a control sequence alias cannot supply a keyword letter.

A literal `t` continues the existing keyword grammar after its already-matched
first word for `to`; a literal `s` does the same for `spread`. The shared keyword
matcher retains only the real matched spelling needed on failure. A successful
clause scans its dimension, opens the group, and consumes the mandatory brace.
A failed prefix restores its real offender and matched prefix, then enters
natural packing and the mandatory-brace grammar. No alternate parser exists
for observed delivery.

Any other first substantive word establishes natural packing. Open the actual
group and consume that word through the mandatory-brace grammar directly. A
brace finishes the prefix; relax continues brace scanning within the open
group; an actual mismatch takes §403's existing diagnostic and backup path.
The initial word was already expanded by the packing probe, as in TeX. Further
expansion needed for the mandatory brace runs only after group entry.

This removes the absent-keyword backups and rereads. It adds no lookahead input
owner, command slot, token publication, virtual old input events, or success
cleanup. Observers describe the new reader's actual operations. Every actual
delivery retains its fuel charge and alignment accounting; removed deliveries
create no synthetic fuel charge or refund.

## Checkpoints and validation

Insertion and adjustment prefixes follow the same ownership rule. TeX82
§1099 scans the insertion class and reports reserved class 255 in the enclosing
group, then calls `new_save_level(insert_group); scan_left_brace`. Only after
the brace does it push internal vertical mode and reset paragraph parameters.
pdfTeX's optional `vadjust pre` keyword also belongs to the enclosing group.
Neither insertion nor adjustment starts an everybox token list.

The native interpreter scans the actual restricted class or adjustment sentinel,
reports before group entry, opens the command-owned insertion group, and scans
the brace with the same processor. It then installs its final active-box owner.
There is no insertion construction record, saved diagnostic context, second
class recovery, or deferred group entry. Full checkpoint restore handles a
resource miss between group entry and body-mode entry just as for ordinary boxes.

Insertion close keeps the same live processor through unsave, packing and
contribution. Page building borrows the command state and renders the §82 input
display only when it reports an actual diagnostic. It must not render and retain
a context string before closing the insertion: most insertions are nested in
boxes or horizontal material where page building does not run. This uses the
page builder's existing live diagnostic interface and adds no frozen context,
deferred callback, or successful cleanup stage.

The live state stack owns the group as soon as entry executes. A resource miss
unwinds the synchronous grammar and the host restores the existing full
checkpoint, including command input, groups, effects and execution owners.
No unfinished construction or resume packet crosses that boundary. Group entry
must not start the new mode early to make brace expansion observe a fabricated
executor state.

Validate the three independent oracle controls, real failed-keyword recovery,
brace aliases, relax and whitespace, packing dimensions, adjusted hboxes,
vertical boxes, vtop and vcenter, tracing, everybox input, resource replay and
full checkpoints. Run the focused ownership tests, full native suite, and
authoritative quality gate, then compare the exhaustive canonical worklist.
Measure frozen identical workloads against the preceding implementation and
pinned pdfTeX. Measurements must distinguish the removed input work from
packing and host setup; they impose no numeric acceptance threshold.
