# umber-interrupt Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the
small native platform boundary for Ctrl-C handling.

## Boundaries

- Keep the public API safe and platform-neutral.
- OS callbacks may only perform lock-free atomic operations; invoke user code
  from the dispatcher thread.
- Keep unsafe FFI local, documented, and covered by native tests where
  practical.
- Do not add TeX engine, CLI, or cancellation policy here.

## File Map

- `AGENTS.md`: crate ownership, safety rules, and file map.
- `Cargo.toml`: target-specific platform FFI dependencies and lints.
- `src/lib.rs`: safe one-time installation API and dispatcher thread.
- `src/unix.rs`: POSIX `sigaction` registration.
- `src/windows.rs`: Windows console-control registration.

## Validation

Run `cargo test -q --tests -p umber-interrupt` and `scripts/check.sh` after
changes.
