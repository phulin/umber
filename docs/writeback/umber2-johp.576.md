# umber2-johp.576 — physical discretionary post-break ownership

Authority: TeX82 `tex.web` §§904, 914, and 918.

A through-ligature automatic discretionary has two distinct post-break views.
The semantic side list contains only the material needed by line breaking.
TeX's physical diagnostic side list owns the reconstituted replacement span
through its synchronization character, including intervening font kerns.

Hyphenation now records physical post-list overrides by exact output position
while constructing each word, then installs them only in `NodeSequence`'s
physical channel. The projection starts with the replacement node, retains
font kerns, and consumes following character provenance up to the physical
replacement count. Semantic nodes, packing, and shipout remain unchanged.
Detailed node dumping marks every emitted post-list line, including breadth
ellipsis lines, rather than only the first child.

Guarded format-loaded TRIP advances the gating log mismatch from byte 49849
to byte 49889. The expected post-branch `BB` ligature, 2-point kern, and `B`
now appear with canonical `..|` ownership. The actual log SHA-256 changes from
`130f4065097ca239c84ed049914ea426b18d666c196db4087f5671b95bc90583` to
`8ef1af7f870397579eeb788f5fb455a5e1a02c45767b6c65ff0ff82533d08ead`, while
normalized DVI and all 22 command events remain exact. The trailing
synchronization-kern ownership is tracked by `umber2-johp.577`.
