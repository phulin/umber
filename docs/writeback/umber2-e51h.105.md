# TeX82 first-dot filename components

TeX82 §§513 and 516--519 define filename components relative to the final area
delimiter: that delimiter resets extension recognition, and the first later
dot begins `cur_ext`. Later dots remain in the extension; they do not move the
earlier extension fragment back into `cur_name`.

`tex-command` applies that boundary while retaining the command profile's
documented host-area delimiter extensions. Packing remains the ordered
concatenation of area, name, and extension.
