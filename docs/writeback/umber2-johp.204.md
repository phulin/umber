# umber2-johp.204 — scalar math fields discard the class nibble

TeX82 §1151 classifies each unbraced scalar field in place and stores only its
character and family as `math_type:=math_char`. The math code's class nibble
does not survive as a noad inside the field. TeX82 §1153 separately stores
every completed braced subformula as `sub_mlist`, even when that list contains
one undecorated Ord noad.

Canonical field execution now preserves that boundary directly: scalar
`MathFieldBody::Character` values become `MathField::MathChar`, while completed
live groups always become `MathField::SubMlist`. Focused scanner and execution
tests cover all seven non-Ord class nibbles, the class-7 current-family rule,
and the absence of replay input events. The
`math/fields-and-atoms` semantic minifixture observes exactly one canonical
expanded-command pair for each non-Ord atom class. The TeX82 catalogue records
this evidence under `tex82.math.fields-and-atoms`.
