# WebAssembly Wire Protocol

Status: schema 1 authority implemented.

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

| Family       | Schema-1 values                                                     | Engine adapter             |
| ------------ | ------------------------------------------------------------------- | -------------------------- |
| Options      | session, project, editor, bibliography, clock, limits, patches      | `src/options.rs`           |
| Resources    | file, font, and PK requests, keys, responses, unavailable variants  | `src/options.rs`           |
| Attempts     | ordinary, project, editor, resource wait, complete, error, status   | `src/result.rs`            |
| Results      | TeX, project, editor, bibliography, generated and output files      | `src/result.rs`            |
| Diagnostics  | stable boundary code, message, source location, bibliography detail | `src/result.rs`            |
| Metrics      | reuse and retained-memory metrics                                   | `src/result/metrics.rs`    |
| Observations | accepted-input ledger, identity, access, phase, owner, outcome      | `src/result/metrics.rs`    |
| Queries      | rendered-source current, deleted, stale, and mismatched results     | `src/result/metrics.rs`    |
| HTML render  | canonical snapshot projection and typed patch plan                  | `src/result/render.rs`     |
| Catalogues   | prepared shard batches, authenticated jobs and misses, named format | `src/catalog_boundary.rs`  |
| Host errors  | authored facade and worker error codes                              | JavaScript facade adapters |

The binding deserializes incoming values into these DTOs, converts them once
to private engine values, converts outgoing engine values once into DTOs, and
serializes them with `serde-wasm-bindgen`. The generated
`src/wire_schema.d.ts` custom section is checked byte-for-byte against
`typescript_declarations()`. The former handwritten TypeScript section and
manual `JsValue` object construction and parsing tables have been deleted.
Catalogue exports likewise return typed DTO objects; JavaScript neither
stringifies input shard batches nor parses returned plans. JavaScript session
and worker orchestration remain separately owned by `umber2-vgjr.13.2`.

Incremental HTML deliberately remains the receiver-migration boundary: the
adapter projects a retained canonical `RenderDocument` snapshot or a
`PatchPlan` directly into the existing schema-1 JavaScript shape. It does not
re-lower artifacts, resolve fonts, or wrap a production patch in the public
Rust receiver envelope. The compatibility audit closed with no released or
external Rust consumer, so that unused envelope and applier were deleted.
JavaScript remains the sole hostile-input, resource-lifetime, and DOM receiver;
moving this projection into derived DTOs must not recreate a second receiver.

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
