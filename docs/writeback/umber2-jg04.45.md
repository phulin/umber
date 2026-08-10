# umber2-jg04.45: checked save-stack high-water

The exhaustive canonical tracer is clean. After null token-list parameters
used their one-word restore form, fresh-cache compatibility TRIP first differed
in TeX82 §1334's final save-stack statistic: the pinned oracle reported `38`
positions while Umber reported `40`. Command, geometry, and normalized-DVI
hashes remained exact.

A hardware watch on the pinned Web2C TeX82 `max_save_stack` showed its final
32-word maximum at `trip.tex` line 296. The raw slots contain three group
boundaries, the three-word vtop specification, twelve two-word
`restore_old_value` records, and one `restore_zero`. TeX then pushes one final
two-word inner dimension restore, but no later checked save operation samples
that completed 34-word live depth.

This is the specified high-water lifecycle. TeX82 §273 checks `save_ptr` before
pushing a group boundary, §275 checks it before pushing either restore form,
and §276 checks it before an aftergroup token. Section 1334 reports that
checked maximum plus the six-word safety margin. Umber instead maximized the
completed live projection after every operation.

The generic projection now returns both live words and the newest physical
record identity. The aggregate owner merges ordering across the environment
journal, typed code-table records, and separately stored aftergroup payloads,
then removes only that newest record to reconstruct the checked pre-push
depth. An ordering-only journal marker makes aftergroup chronology explicit
without changing its semantic payload owner. Focused controls cover group,
ordinary restore, code-table restore, and aftergroup pushes, plus a global
definition that must not create a record.

Fresh-cache compatibility TRIP is now exact, including its `38` save-stack
positions and all compared artifact hashes. No comparison or normalization
changed.
