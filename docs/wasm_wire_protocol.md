# WebAssembly Wire Protocol

Status: schema 1 authority and migration contract.

The host-neutral DTOs in `crates/umber-wasm/src/wire.rs` are the structural
authority for values crossing the Rust--JavaScript boundary. They contain only
owned strings, numbers, byte buffers, vectors, options, and other wire DTOs.
They do not expose engine sessions, VFS paths, state handles, artifacts, or
diagnostic implementations. Binding adapters convert between DTOs and those
private types.

`wireSchemaVersion()` returns the compatibility version. A breaking field,
discriminant, representation, validation, or omission change increments it.
Adding an optional field does not: schema 1 readers intentionally ignore
unknown object fields. Missing required fields and unknown discriminants remain
errors.

## Inventory and ownership

| Family       | Schema-1 values                                                     | Existing adapter during migration           |
| ------------ | ------------------------------------------------------------------- | ------------------------------------------- |
| Options      | session, project, editor, bibliography, clock, limits, patches      | `src/options.rs`                            |
| Resources    | file, font, and PK requests, keys, responses, unavailable variants  | `src/options.rs`, `src/result/resources.rs` |
| Attempts     | ordinary, project, editor, resource wait, complete, error, status   | `src/result.rs`                             |
| Results      | TeX, project, editor, bibliography, generated and output files      | `src/result.rs`                             |
| Diagnostics  | stable boundary code, message, source location, bibliography detail | `src/result.rs`                             |
| Metrics      | reuse and retained-memory metrics                                   | `src/result/metrics.rs`                     |
| Observations | accepted-input ledger, identity, access, phase, owner, outcome      | `src/result/metrics.rs`                     |
| Queries      | rendered-source current, deleted, stale, and mismatched results     | `src/result/metrics.rs`                     |
| Host errors  | authored facade and worker error codes                              | `js/compile.js`, `js/worker-controller.js`  |

The manual adapters and `typescript_custom_section` remain compatibility
predecessors until `umber2-vgjr.13.4` switches every family together and
deletes them. The DTO module establishes the replacement authority without
creating a second engine model. JavaScript session and worker orchestration
remain separately owned by `umber2-vgjr.13.2`.

## Representation invariants

- Every integral DTO field whose Rust range can exceed 32 bits uses
  `SafeInteger`; values above `2^53 - 1` are rejected. Existing manual option
  parsing applies the same bound.
- Byte fields use `serde_bytes`, an explicit TypeScript `Uint8Array` override,
  and the non-JSON `serde-wasm-bindgen` serializer. The serializer emits a
  `Uint8Array`; no byte field uses base64, a JSON number array, or an
  intermediate JSON string.
- `None` properties governed by `skip_serializing_if` are absent, not `null`.
  This distinction is part of the golden shape.
- Stable boundary error codes are closed kebab-case enums. Bibliography
  diagnostic codes remain strings because their compatibility authority is the
  bibliography subsystem.
- Request and response vectors preserve their input order. DTO conversion must
  not sort, deduplicate, or apply engine policy.

Rust shape tests cover TypeScript derivation, omission, safe integers, unknown
fields, and error spelling. The WASM golden constructs the actual JavaScript
value and proves its byte fields are `Uint8Array` instances rather than JSON
copies. The existing package and worker tests continue to own transfer,
cancellation, timeout, and containment behavior.
