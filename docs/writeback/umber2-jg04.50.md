# umber2-jg04.50: Transient Node-List Allocator Extent

The exhaustive canonical tracer is clean. After pre-restore high-water
sampling advanced official e-TRIP to `1999` words, the pinned Web2C allocator
was watched directly during the format-loaded phase. `lo_mem_max` changes from
`1020` to `2020` at e-TRIP line 528 with `var_used=1008`. The allocating stack
is `get_node` through `new_null_box`, `char_box`, `stack_into_box`,
`var_delimiter`, `make_left_right`, and `mlist_to_hlist` while the deeply nested
`\middle` formula is still being converted.

TeX82 §§125--127 establish the split allocator and the 1000-word low-memory
growth block. Sections 682--683 allocate math nodes during `mlist_to_hlist`,
and §1334 reports the inclusive allocator coordinates after temporary lists
have been released. The root is therefore the missing construction-time
observation, not the final box-register closure or e-TeX's later sparse-array
tests.

The generic fix projects each node list at the existing typed freeze boundary,
including its physical child lists, before immutable storage can detach it
from the active construction. The permanent allocator high-water absorbs the
projection; semantic reachability and format identity remain unchanged. A
focused positive control proves that 501 transient two-word penalty nodes grow
the low arena even when never installed in an environment cell. Its negative
control uses 501 §135 character nodes and proves that the same list length does
not invent a low-memory block. The four independent §125 scratch positions are
not charged again at this construction boundary.

Exact compatibility TRIP remains green. Official e-TRIP advances from `1999`
to `2999` words with exact normalized DVI and zero projected semantic
divergences. The independent missing 318-word high-arena extent is recorded
observed-only as `umber2-jg04.51`.
