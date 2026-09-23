# State responsibility boundaries

This pass organizes methods around the existing owners. It does not change
their storage, admission, or checkpoint authority.

| Owner            | Retained authority                                                                       | Method groups                                                                                                              |
| ---------------- | ---------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `World`          | One host backend, effect and input ledgers, and `WorldSnapshot`                          | `input_dependencies`, `effect_publication`, and `checkpoint_lifecycle`                                                     |
| `ForkArena`      | One typed lane of logical coordinates, topology, and settlement metadata                 | `checkpoint_lifecycle`, `batch_transfer`, and `whole_region_transfer`; physical `ChunkPool` remains the sole payload owner |
| `CommandContext` | One already-admitted borrow of session, retained generation, dense core, and page region | `pdf_commands`, `page_material`, and `page_builder`                                                                        |

The child modules contain inherent methods on these same types. They add no
service, owner, snapshot, or conversion. `Universe` still admits and settles
the aggregate checkpoint. `tex-dense-prefix` still owns raw initialized
superblocks, `tex-dense-arena` still owns physical block identities and safe
logical tables, `ChunkPool` still owns page payload, and `ForkArena` still owns
TeX list topology and lane lifetime. Page list reads continue through borrowed
views and cursors.

Validation must cover same-lineage rollback, candidate accept/reject, whole
batch transfer, command page/PDF access, and the existing feature and compile
fail gates. No wire format, public signature, or test selection changes.
