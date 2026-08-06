# umber-fetch Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns
native host policy for bounded verified blob persistence and HTTPS distribution
acquisition. It does not own TeX format identity or engine validation.

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

- `src/cache.rs`: one bounded `BlobStore`, `VerifiedBlobSpec`, platform cache discovery, compatibility readers, and the locked entry state machine for validation, quarantine, migration, construction, and publication.
- `src/blob_store_unix.rs`: root-handle-relative Unix authority, per-key process locks, durable quarantine, and no-clobber publication for every blob.
- `src/blob_store_unsupported.rs`: fail-closed native persistence boundary for hosts without the required anchored I/O primitives.
- `src/distribution_client.rs`: store-owning native distribution acquisition façade for verified manifest and object batches.
- `src/downloader.rs`: sole policy-parameterized bounded HTTPS download, retry, cancellation, length, and digest verification authority.
- `src/fetch.rs`: bounded blocking object batching, cache coordination, and diagnostics.
- `src/manifest.rs`: manifest-specific policy and public error adapter for the shared downloader.
- `src/lib.rs`: public native cache/fetch contract.
- `src/tests.rs`: cache and local fixture-server contract tests.
- `src/tests/fixture.rs`: socket-free in-memory HTTP fixture transport.

## Validation

Run `cargo test -q -p umber-fetch --tests`, then the workspace format/clippy
gate. Tests must remain hermetic and use only loopback fixture servers.
