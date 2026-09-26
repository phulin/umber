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
the old scalar for reverse-order rollback. If a retained checkpoint still
needs the old value of that current owner, the first write moves the old owner
to history and gives the live cell an independent copy before changing its
root. A group save does not turn an unscoped dimension write into a local
assignment: it already owns the box from before the local binding. In
particular, mutating a box through an unchanged outer binding inside a group
persists after that group closes.

The ownership decision uses the current cell's checkpoint and group save
state. It does not scan the box closure or count node references. The only
mutable storage seam is a validated fixed annex word belonging to the live
durable region. No page-root copy is required for an ordinary dimension
write, and no node alias, refcount, or copy-on-write owner is introduced.
