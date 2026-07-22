# TeX Live WASM Publisher Guidance

This standalone tool publishes browser distribution inputs. Its output must be byte-for-byte reproducible for identical pinned roots, regardless of directory enumeration order or host path. Never put source root paths in the manifest.

All accepted source paths are normalized relative POSIX paths. Reject symlinks, non-UTF-8 paths, traversal, backslashes, and physical case-fold path collisions. Logical lookup keys remain case-sensitive because TeX Live contains case-distinct basenames in separate paths. Lookup precedence is configured root order followed by normalized path order. Verify the complete supported-file tree digest before writing output.

Object names are derived solely from SHA-256. Manifest serialization uses ordered maps and one trailing newline. Dependency entries are hints and must refer to valid logical keys, but are not required to be transitively complete.

Schema-3 format entries may carry schema-1 input closures. Canonicalize their
request keys, enforce the shared count/key-size bounds, reject duplicates, and
verify every key against the complete published file map before writing. Keep
schema-1 format metadata as the legacy no-closure form; schema-2 format
metadata requires a closure.

Production TeX lookup objects include every file below `tex/`, TFM metrics,
and the runtime font areas `afm`, `enc`, `map`, `opentype`, `pk`, `type1`,
`truetype`, and `vf`. Documentation and source trees are never publishable.
Non-TFM runtime files use the manifest's `tex:` request kind because that is
the current remote resolver vocabulary; Umber-native formats remain separate
manifest format objects.

The separate `html` publication profile emits root schema 4 and shard schema
2. It verifies the complete configured roots but selects only explicit runtime
keys plus every selected format's authenticated closure. Its TEXMF inputs are
limited to `tex/` and TFM; WOFF2 and license objects come from an exact
schema-2 catalog/object-source set. Never weaken this allow-list to admit VF,
AFM, ENC, maps, PK, Type 1, or SFNT transport objects. HTML inventory ceilings
are independent of the full snapshot.

## File map

- `src/lib.rs`: publication orchestration and public configuration.
- `src/sharded.rs`: canonical root/shard schema, partitioning, writing, and staged verification.
- `src/scan.rs`: deterministic root scanning, pin verification, and precedence.
- `src/tlpdb.rs`: TeX Live runfile ownership and bounded package dependency-hint derivation.
- `src/tests.rs`: fixture publication, collision, path, and precedence tests.
- `src/main.rs`: small JSON-config command-line entry point.
