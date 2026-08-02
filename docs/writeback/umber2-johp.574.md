# umber2-johp.574 — physical discretionary pre-break branch spans

Authority: TeX82 `tex.web` §§904, 914, and 918.

At an automatic hyphenation point synchronized through a font kern, TeX's
physical linked-list representation places the preceding reconstituted glyph
span in the discretionary's pre-break branch. Umber's compact semantic node
keeps only the hyphen there, which is correct for line breaking and shipout
but incomplete for detailed box diagnostics.

Paragraph hyphenation now returns a paired `NodeSequence`. Its semantic
channel is unchanged; its physical channel reconstitutes the preceding
character or ligature span with the hyphen for boundary discretionaries. Box
reporting retains a separate short-display projection with the semantic
pre/post side lists, while detailed `show_box` traversal consumes the richer
physical branch. A focused regression pins the physical branch without
changing the semantic frozen list.

Guarded format-loaded TRIP advances the gating log mismatch from byte 49716
to byte 49757. The expected `AA` ligature, 3-point kern, and hyphen branch is
now exact. The actual log SHA-256 changes from
`894e7b4d51619aba60cd79b4260a5c476228ae6591107dfa71ddfac9a6238360` to
`991f8dfff55c04b1c7754f034359074d862a2590b8e0368fb9c6c77f01af7e86`, while
normalized DVI and all 22 command events remain exact. The following physical
node-order front is tracked by `umber2-johp.575`.
