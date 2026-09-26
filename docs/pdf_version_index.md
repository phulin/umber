# PDF version index storage

The PDF ledger retains old lookup roots for checkpoints and candidate
transactions. Each version write must publish a new root without changing any
node reachable from an older root. The earlier binary trie copied 64 branch
nodes plus a leaf for every write, even when the ledger had only one key. The
book heap audit attributes sustained allocation growth to that index.

The index is a persistent Patricia tree over the existing packed `u64` keys.
A leaf stores its complete key and current version event. A branch stores the
most significant bit that distinguishes its two children. Branch bits strictly
decrease from root to leaf, so there are no unary paths. Lookup follows the
key's bits and verifies the complete key at the leaf. An update to an existing
key appends one replacement leaf and copies only its branch ancestors. A new
key first finds a representative leaf, then adds one leaf and one branch at
the highest differing bit, copying only the ancestors above that branch.

Nodes remain in append-only accepted and candidate vectors with stable `u32`
coordinates. A candidate may point to accepted nodes; rejection discards only
candidate nodes and restores the accepted root, while acceptance appends the
candidate vector to the accepted vector without changing coordinates. Root
restoration and historical lookup therefore do not depend on mutable owner
bits, reference counts, or first-write copies. An index with `K` distinct keys
has `2K-1` reachable nodes at one root; the retained history cost per write is
bounded by its actual branch depth rather than the 64-bit key width. Tests
cover sparse high-bit keys, updates and old roots, candidate accept/reject,
and initialized-node and reserved-vector growth against distinct-key count.
The checkpoint memory estimate uses vector capacity for this index, since
unused reserved slots still occupy heap allocation.
