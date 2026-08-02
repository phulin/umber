# umber2-johp.575 — physical discretionary containing-list traversal

Authority: TeX82 `tex.web` §§174, 182, and 904.

Detailed node-list display does not suppress the physical nodes counted by an
automatic discretionary's replacement count. The count describes linked-list
topology; those nodes remain part of `show_node_list` traversal. At a boundary
font kern, only the discretionary's position changes relative to Umber's
structured ligature representation: the discretionary renders first, followed
by the ligature and kern.

Physical diagnostic traversal now applies that single boundary reorder and
otherwise renders every containing-list node. Its breadth accounting includes
all three reordered nodes. Post-break side-list markers replace the final
indentation dot, matching TeX's `..|` prefix at nested depths. Two bounded
tests cover an ordinary physical replacement span and consecutive boundary
and through-ligature discretionaries.

Guarded format-loaded TRIP advances the gating log mismatch from byte 49757
to byte 49849. The following ligature and kern now appear in canonical order,
and the next post-break marker has canonical indentation. The actual log
SHA-256 changes from
`991f8dfff55c04b1c7754f034359074d862a2590b8e0368fb9c6c77f01af7e86` to
`130f4065097ca239c84ed049914ea426b18d666c196db4087f5671b95bc90583`, while
normalized DVI and all 22 command events remain exact. The newly exposed
physical post-break branch projection is tracked by `umber2-johp.576`.
