# umber-fetch Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
native host policy for persistent distribution caching and HTTPS acquisition.

## Boundaries

- Keep filesystem, environment, threading, and network access in this crate;
  engine crates, `umber-vfs`, and `umber-distribution` must remain I/O-free.
- Treat manifest digests and byte counts as untrusted declarations: enforce
  limits before reading bodies and verify every cached or downloaded byte.
- Never return a partially acquired batch. Cache population may survive a
  failed batch, but callers receive bytes only when every request succeeds.
- Production object URLs must use HTTPS. Plain HTTP is accepted only for
  loopback fixture servers.

## File Map

- `src/cache.rs`: platform cache discovery and verified atomic blob storage.
- `src/fetch.rs`: bounded blocking batch acquisition, cooperative cancellation, retry, and diagnostics.
- `src/format_cache.rs`: canonical generated-format identity and validated atomic schema-10 entry storage.
- `src/format_cache/tests.rs`: format-cache identity, validation, recovery, and concurrency tests.
- `src/manifest.rs`: cancellable bounded HTTPS manifest download and trust-pin verification.
- `src/lib.rs`: public native cache/fetch contract.
- `src/tests.rs`: cache and local fixture-server contract tests.

## Validation

Run `cargo test -q -p umber-fetch --tests`, then the workspace format/clippy
gate. Tests must remain hermetic and use only loopback fixture servers.
