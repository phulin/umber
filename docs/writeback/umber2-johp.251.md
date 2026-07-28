# umber2-johp.251 — exact alphabetic constant bound

TeX82 §442 accepts a backtick character constant through the complete eight-bit
domain, including 0 and 255. An exact-profile raw character or one-character
control sequence above 255 instead reports `Improper alphabetic constant`,
sets the scanned integer to zero, and backs the offending token up; the
following optional space therefore remains unconsumed. The separately named
UnicodeExtended profile retains its widened scalar result without that error.
