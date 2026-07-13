# TeX Live WASM Publisher Guidance

This standalone tool publishes browser distribution inputs. Its output must be byte-for-byte reproducible for identical pinned roots, regardless of directory enumeration order or host path. Never put source root paths in the manifest.

All accepted source paths are normalized relative POSIX paths. Reject symlinks, non-UTF-8 paths, traversal, backslashes, and case-fold collisions. Lookup precedence is configured root order followed by normalized path order. Verify the complete supported-file tree digest before writing output.

Object names are derived solely from SHA-256. Manifest serialization uses ordered maps and one trailing newline. Dependency entries are hints and must refer to valid logical keys, but are not required to be transitively complete.

## File map

- `src/lib.rs`: publication orchestration and public configuration/schema.
- `src/scan.rs`: deterministic root scanning, pin verification, and precedence.
- `src/tests.rs`: fixture publication, collision, path, and precedence tests.
- `src/main.rs`: small JSON-config command-line entry point.
