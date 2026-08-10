# umber2-jg04.42: box-spec save-stack projection

The exhaustive canonical tracer is clean. After control-sequence accounting
became exact, fresh-cache compatibility TRIP first differed in §1334's final
save-stack statistic: the pinned oracle reported `38` positions while Umber
reported `36`. Command, geometry, and normalized-DVI hashes remained exact.

A hardware watch on the pinned Web2C TeX82 `max_save_stack` showed its first
32-word high-water at `trip.tex` line 296. TeX82 §645's `scan_spec` stores a
packing kind and dimension immediately below a new group boundary. Section
1083 calls it with `three_codes=true` for ordinary hbox, vbox, and vtop bodies,
so their `box_context` is a third saved word. Umber projected only two words
for every live box body.

The generic projection now derives the below-boundary word count from each
live box kind. Ordinary boxes contribute three words, §1167's vcenter
contributes only the packing pair, and §1099's insert/vadjust group contributes
none. Those three classes are pinned as focused positive and negative controls.

Fresh-cache TRIP advances from `36` to `37` save-stack positions. The remaining
independent one-word difference is recorded observed-only as
`umber2-jg04.43`; this issue does not absorb it.
