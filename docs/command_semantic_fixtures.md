# Committed Command Semantic Fixtures

Status: repository contract v1 for canonical command-core fixtures.

## Authority and boundary

Committed command fixtures are expectations produced only by pinned,
transparently instrumented TeX82, e-TeX 2.6, or pdfTeX 1.40.27 builds. Umber
never generates, rewrites, or blesses their expected values. Live reference
execution occurs only through `scripts/regen-fixtures.sh`; Cargo correctness
tests load committed files and require no TeX-family executable, source tree,
network, clock, or host tool.

The semantic event schema and the repository fixture contract are separate
versions:

- `tex-oracle` schema 1 defines normalized semantic events and the inner
  oracle-manifest identity written into the JSON Lines header.
- fixture contract 1 binds that stream to a repository selector, canonical
  profile and tools, mandatory source citations, focused INITEX sources, and
  committed ordinary artifact observations.

This separation preserves the existing schema-v1 manifest preimage. A fixture
contract revision cannot silently reinterpret a schema-v1 event stream.

## Layout

One fixture is one directory under:

```text
tests/corpus/command/<engine>/<fixture>/
    manifest.json
    events.jsonl
    sources/
    outputs/
```

`manifest.json` is compact canonical UTF-8 JSON followed by LF. Struct fields
use contract order and maps use bytewise key order. Unknown fields, alternate
whitespace, missing final LF, unsafe logical names, and unsupported versions
are rejected.

`events.jsonl` uses schema 1. Its first line contains the schema and the
SHA-256 identity of the inner oracle manifest. Later lines contain contiguous
zero-based sequence numbers and normalized semantic events. Normalization
only converts CRLF or CR inside semantic strings to LF; it does not reorder,
filter, rename, or otherwise hide semantic differences.

## Manifest contract

Fixture contract 1 records:

- `contract`: exactly `1`;
- `name`: the stable regeneration selector;
- `profile`: the canonical invocation and character profile;
- `oracle`: the existing schema-v1 `Manifest`;
- `tools`: canonical translator/tool names, versions, and SHA-256 identities;
- `citations`: canonical WEB source, narrow section or procedure, and the
  observed semantic boundary;
- `sources`: logical input name to committed path, byte length, and SHA-256;
- `events`: committed stream path, byte length, and SHA-256; and
- `outputs`: ordinary channel to committed path, byte length, and SHA-256.

The inner oracle manifest independently records the event schema, engine
dialect and banner, canonical WEB source, ordered upstream changes, final
instrumentation change, logical input identities, deterministic environment,
epoch, clock, random seed, distribution identity, and ordinary-output hashes.
The outer source and output identities must be an exact one-to-one match with
the inner maps.

Paths are relative logical names. Absolute paths, backslashes, empty
components, and `..` components are forbidden. Host paths, executable
allocation identity, TeX `mem` addresses, pool indexes, selector state, and
input-stack indexes never enter a fixture.

Every fixture has at least one canonical citation. Citations name the
narrowest stable WEB procedure or section that owns the observed transition;
instrumentation helper names are not authorities. Every event source location
must name a declared fixture source and use a nonzero line.

## Ordinary artifact observations

Semantic events do not replace ordinary reference behavior. Each committed
fixture also records the relevant clean/instrumented transparency channels,
such as terminal bytes, the explicitly documented normalized log, exit
status, generated file bytes, DVI bytes, or deterministic PDF/projection bytes
where applicable.

The fixture manifest binds exact bytes and hashes for every declared channel.
The corresponding inner oracle output map must contain exactly the same
channels and hashes. No cargo test invokes a reference executable to recreate
a missing observation.

## Validation and correctness consumption

The dependency-light `tex-oracle` crate owns parsing and validation:

```bash
cargo run -q -p tex-oracle --bin tex-oracle-validate -- \
  --fixture tests/corpus/command/tex82/command-transitions-v1 \
  --semantic-matrix tests/tex82-oracle/semantic-event-matrix.txt \
  --audit-matrix tests/tex82-oracle/fixture-audit-matrix.txt
```

The complete committed TeX82 command suite has one deterministic, offline
entry point. It inventories every directory under `tests/corpus/command/tex82`,
requires an exact corresponding `fixture` and `fixture-audit` record in the
regeneration contract, verifies the pinned manifest and matrix identities,
then validates each manifest-bound ordered stream and coverage audit:

```bash
cargo run -q -p tex-oracle --bin tex-oracle-validate -- --tex82-command-suite
```

The report lists each selector, canonical profile, inner schema-v1 oracle
identity, event count, and every covered `family/boundary` command seam in
bytewise order. It reads only committed files; it never builds, invokes, or
consults a TeX engine or legacy Umber. A missing fixture record, unregistered
fixture directory, stale identity, profile mismatch, event-order failure, or
uncovered matrix requirement fails the command.

`CommittedFixture::load` verifies canonical manifest encoding, contract and
schema versions, canonical profile, required tools and citations, file
lengths and hashes, source/output agreement with the inner manifest,
canonical JSON Lines encoding, contiguous event sequences, stream-to-manifest
identity binding, and declared source locations.
`CommittedFixture::audit_matrices` additionally requires every executable
matrix behavior to occur in the committed stream, every matrix source to be a
declared focused source and vice versa, and every behavior family to own
canonical citations and ordinary observations. It rejects unowned citations,
outputs, sources, or behavior families. The regeneration contract pins both
matrix identities, so `--validate-only`, Cargo tests, and live regeneration
consume the same audit. Oracle regeneration contract 2 adds these paired
matrix identities without changing fixture contract 1 or semantic-event
schema 1.

Correctness tests call that API directly on committed directories. Tests may
compare a future Umber observer stream with `CommittedFixture::stream`, but
they may not replace fixture data, derive expectations from Umber, or fall
back to a live engine.

The test-gated `tex-command` fixture registry first runs the same complete
offline TeX82 suite validation, then loads every registered fixture source in
logical-name order. Each source is registered as immutable command input under
the manifest-pinned `TeX82 + EightBitExact` INITEX profile and replayed from a
fresh explicit command state through `CommandProcessor::get_next`. The adapter
captures only command-owned observer records and derives a finite delivery
bound from the registered source bytes. It does not add another lexer,
expander, executor, fixture generator, or production dependency on
`tex-oracle`; ordered schema comparison remains a separate test-only layer.

The offline comparison runner is deliberately a host tool rather than a
`tex-command` production dependency:

```bash
cargo run -q -p tex-command-stream -- --repository .
```

It validates the complete registered suite, constructs the explicit canonical
INITEX startup state, scans the terminal filename `transitions.tex`, and opens
that root above terminal input before later `\\input` capability work can
register children. Terminal and root receive deterministic source identities
in that order; fixture-directory or logical-name iteration never selects the
root. It translates command-owned observer records at the host boundary and
compares every produced event against the complete committed stream without
filtering expected events. A mismatch reports its fixture
identity, ordered index, typed expected and actual values, and available
source/provenance context; output is bounded. The current command-core scope
may therefore report an early divergence while implementation slices are
incomplete. Its comparator self-test covers exact quiet matches and a
deterministic injected earliest mismatch.

## Regeneration selection

The representative selector is pinned by
`tests/oracle-regeneration-manifest.txt`:

```bash
scripts/regen-fixtures.sh \
  --oracle tex82 \
  --profile initex-eight-bit \
  --fixture tex82/command-transitions-v1 \
  [--offline]
```

The workflow first completes the pinned clean/instrumented TeX82 transparency
build. It validates the committed fixture, compares the focused source and
every ordinary output with the clean run, replaces only the live observer's
documented all-zero unbound header in a temporary candidate with the committed
inner-manifest identity, and then requires the candidate stream to be
byte-identical to `events.jsonl`. The live unbound stream is never committed.

`--validate-only` validates the source/regeneration manifests, both pinned
coverage matrices, and committed fixture without acquiring or executing the
reference engine. TeX82 and aggregate validation always include this audit;
the fixture selector remains valid only with its exact engine and profile, so
cross-engine or unknown selections fail.

The first representative fixture is
`tests/corpus/command/tex82/command-transitions-v1`. It uses focused,
font-independent INITEX sources and commits the complete TeX82-applicable
schema-v1 transition matrix plus terminal, normalized-log, status, DVI, and
generated-effect observations. Its input-focused sources isolate M/N/S
tokenization, ignored/invalid/comment/end-line handling, `^^` notation,
`get_token`, nested parameter replay, legal source retirement, and each
non-normal scanner-status EOF recovery. Outer-validity diagnostics carry the
canonical live scanner status as a typed name, and the trace records the exact
inserted right-brace, `\par`, frozen `\cr`, and frozen `\fi` tokens. The main
source explicitly assigns `\year`,
`\month`, `\day`, and `\time` before shipout because canonical TeX82's
`onlyTeX` host boundary does not honor Web2C's `SOURCE_DATE_EPOCH`; this makes
the ordinary DVI preamble exact across regeneration runs.
Its `expansion-macros.tex` child separately exercises `get_x_token`,
`\noexpand`, `\expandafter`, `\csname`, conversion primitives, canonical
macro matching through nine parameters, paragraph and overlap recovery,
nested parameter replay, definition forms and prefixes, ordinary and expanded
`scan_toks`, and direct `\the` splices. Committed `\meaning` and `\show`
transcript bytes independently expose the representative resulting meanings.
Its `scanner-conditionals.tex` child separately exercises signed radix
integers, fractional physical dimensions, infinite-order glue, typed internal
integer/dimension/glue/token-list values, and reference-visible `\the`
spellings. The same source covers condition push/change/branch/pop lifecycle,
`\if`, `\ifcat`, raw-operand `\ifx`, `\ifcase` progress, nested conditions
inside both evaluation and `pass_text`, and skipped balanced braces. Inserted
relax, extra-delimiter, and EOF-during-skip recovery remain visible in the
normalized transcript; `scanner-conditionals-eof.tex` isolates the last case.
Its `alignment-delivery.tex` child separately exercises preamble lifecycle and
repetition, literal-brace `align_state`, control-sequence group aliases, tab,
`\span`, `\cr`, and `\crcr` interception, u/v/omit-template delivery and
retirement, `\noalign`, nested suspend/resume ownership, backup correction,
and preamble, brace, and extra-tab recovery. Transcript messages independently
expose template and no-align execution; shipped font-independent rules bind
successful alignment packaging into the committed DVI observation.
Its `off-save.tex` child isolates executor recovery without using Umber as an
oracle: an `\endgroup` above bottom level is backed up, paired with an inserted
right brace, and replayed; the replay then reaches bottom level and is dropped.
The following message is a bounded-progress observation. Schema-v1 diagnostic
events distinguish the two `off_save` branches, while the existing recovery
events retain the exact replay and inserted-closer tokens.
