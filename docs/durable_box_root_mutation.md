# Durable box root mutation and destructive transfer

TeX82 §1079's `begin_box` takes the pointer from `box(n)` and clears that
register at the current level. The paired `copy_code` calls
`copy_node_list(box(n))`; §1106's `unpackage` makes the same distinction for
the child list after checking box kind and mode. TeX82 §1055's
`alter_box_dimen` writes one width, height, or depth word of the existing box
node. The assignment prefix does not make that write a scoped register
assignment.

Umber's durable box cell names one exclusive owner slot. A group save owns a
different, older box only after the first local assignment to that cell. An
open group without that save does not retain the current owner. Likewise, a
checkpoint journal owns the pre-change value only after its first write, and
an operation journal owns an inverse until commit. Those retained roots, not
group depth alone, determine whether destructive `\box`, `\unhbox`, or
`\unvbox` can loan the current owner to the page. A unique owner transfers
through `loan_durable_to_page_in_place` while the operation is active; rollback
returns the same region to its reserved slot. A current owner needed by a
retained checkpoint must be preserved before consumption, and only that case
requires a history-preservation copy. Explicit `\copy`, `\unhcopy`, and
`\unvcopy` always copy recursively.

Box dimension writes mutate exactly one scalar in the root box payload. The
durable owner and all child coordinates remain the same. An operation records
the old scalar for reverse-order rollback. A retained checkpoint records the
first old value of each dimension of its then-current box; repeated writes in
one checkpoint epoch update the same root without another closure copy or
checkpoint entry. A stable semantic lineage distinguishes a historical copy
of that box from a later unrelated box assigned to the same register.

On checkpoint restore, binding inverses run first, then scalar inverses apply
only to the restored box lineage. This order lets an edited box be overwritten
or destructively consumed before restore: its checkpoint-owned historical
copy receives the old dimension after it becomes current again. A candidate
fork copies a closure only when accepted and candidate branches both need the
same current owner with different scalar values; acceptance or rejection
retires the losing branch. A group save does not turn an unscoped dimension
write into a local assignment: it already owns the box from before the local
binding. Mutating a box through an unchanged outer binding inside a group
therefore persists after that group closes.

The ownership decision uses the current cell's checkpoint and group save
state. It does not scan the box closure or count node references. The only
mutable storage seam is a validated fixed annex word belonging to the live
owner's region. No page-root copy is required for a box dimension write, and
no node alias, refcount, or copy-on-write owner is introduced.

The automatic output box 255 is page-owned. Page fire-up constructs its root
wrapper after the retained checkpoint boundary, and execution enters the
output group before assigning a dimension to it. Runtime checkpoints require
level zero, so none can retain that wrapper while it is edited; its children
may still belong to older page chunks. The page owner edits only the
exclusive wrapper annex word and refreshes the root's semantic identity. An
operation-only scalar inverse captures its stable page coordinate. Rollback
applies that inverse before page suffix truncation, even if the output root
has meanwhile been consumed or replaced. A PageBuilder transaction may
restore the root first, so the inverse updates the live root identity only
when its coordinate still matches. A shared wrapper annex rejects mutation
rather than acquiring implicit copy-on-write ownership.

An operation mark records durable-box suffix lengths and scalar nesting
depth without pushing an empty journal frame. Only a mutation appends undo
work; nested marks still settle in last-in-first-out order. Transfer loans,
scalar edits, and binding changes must replay in reverse event order so a
page-to-durable transfer followed by a dimension edit restores the scalar
before returning the box to its page owner.
