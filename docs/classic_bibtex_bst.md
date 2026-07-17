# Classic BibTeX and `.bst` execution

Status: proposed design and implementation plan

This document defines a host-neutral classic BibTeX backend for Umber. The
backend reads LaTeX auxiliary files, BibTeX datasources, and executable `.bst`
style programs, then produces classic `.bbl` and `.blg` files in-process on
native and WebAssembly targets. It integrates with the existing Biber-compatible
bibliography subsystem and transactional multipass project orchestration without
forcing classic BibTeX semantics through the Biber data model.

The central architectural decision is:

> Classic BibTeX and Biber are separate semantic backends behind one resource,
> result, and project-orchestration facade. They share lossless input and host
> infrastructure where behavior is genuinely common, but they do not share
> selection, graph, sorting, labeling, or output semantics.

## 1. Motivation and terminology

Umber currently implements the backend used by biblatex: a pinned
Biber-compatible transformation from `.bcf` control data and bibliography
datasources to a typed processed bibliography and a biblatex-format `.bbl`.
Classic BibTeX has a materially different execution model:

```text
Biblatex/Biber:
    .bcf + .bib -> typed processing stages -> biblatex .bbl

Classic BibTeX:
    .aux + .bib + executable .bst -> stack-machine execution -> classic .bbl
```

A `.bst` file is a program, not a passive formatting description. It declares
entry fields and variables, defines functions, chooses when the database is
read, iterates or reverses entries, assigns sort keys, sorts, and writes output
through observable `write$` and `newline$` operations. Consequently, `.bst`
support is not an output format for `bib-output` and is not an alternate style
configuration for `bib-sort` or `bib-label`.

This document uses:

- **biblatex backend** for the existing pinned Biber-compatible pipeline;
- **classic backend** for the proposed BibTeX-compatible `.aux`/`.bst` engine;
- **style compiler** for `.bst` parsing, validation, symbol resolution, and
  lowering to an immutable executable program; and
- **style VM** for bounded execution of that program over a classic database.

The first compatibility target is classic BibTeX 0.99d as shipped by the
pinned TeX Live source under `third_party/texlive-source/src/texk/web2c`.
The compatibility identity covers the merged `bibtex.web` plus `bibtex.ch`
program, the WEB2C configuration and build flags, and the applicable kpathsea
file-resolution contract. `bibtex.web` alone is not the executable
specification: the change file alters capacities, character handling, line
wrapping, logging, exit status, command-line behavior, and file lookup.
BibTeX8, BibTeXu, pBibTeX, and upBibTeX provide useful supplemental fixtures
but are not part of the initial semantic promise. Extensions must be exposed
as explicit compatibility identities rather than silently changing classic
behavior.

## 2. Goals

The classic backend must:

- implement the complete pinned classic `.bst` language without a subprocess;
- run with the same pure-Rust implementation in native and WASM builds;
- reproduce the pinned reference's `.aux`, `.bib`, `.bst`, `.bbl`, and `.blg`
  behavior for the committed compatibility corpus;
- use immutable `umber-vfs` snapshots and typed batched resource requests;
- preserve observable command, entry, stack, diagnostic, and output order;
- bound input bytes, tokens, symbols, strings, stack depth, call depth,
  instructions, entries, diagnostics, output bytes, and total work;
- cache parsed auxiliary files, parsed datasources, and compiled styles by
  content identity without making cache hits semantically observable;
- integrate with transactional TeX-bibliography-TeX convergence;
- retain the existing Biber APIs and behavior for current callers; and
- expose enough read-only classic state for diagnostics and tooling without
  pretending it is a Biber `ProcessedBibliography`.

## 3. Non-goals

The initial implementation does not:

- translate `.bst` programs into Biber templates or biblatex `.bbx`/`.cbx`
  styles;
- run a system `bibtex` executable or search a host TeX installation from a
  semantic crate;
- support BibTeX8 character-set files, BibTeXu ICU collation, Japanese BibTeX
  variants, or implementation-specific extensions unless separately enabled;
- apply Biber sourcemaps, data-model validation, xdata processing, sorting,
  labels, uniqueness, or serialization to a classic job;
- expose the style VM as a general-purpose programmable runtime;
- promise reference-compatible resource exhaustion at the reference program's
  historical fixed array sizes; or
- preserve partial generated files after a failed transactional attempt.

Reference-compatible diagnostics at a safe Umber limit are required. The
exact historical point at which an undersized reference array overflows is not.

## 4. Architecture

The two backends converge only at the public orchestration boundary:

```text
                         bibliography facade
                 jobs, attempts, resources, results
                                |
           +--------------------+--------------------+
           |                                         |
     biblatex backend                          classic backend
     .bcf control                              .aux control
           |                                         |
input -> graph -> sort -> label             aux + raw .bib + .bst
           |                                         |
       bib-output                              style compiler
           |                                         |
  biblatex .bbl/.blg                              style VM
           |                                         |
           +--------------------+--------------------+
                                |
                detached files and diagnostics
                                |
                 LatexProjectSession convergence
```

The existing `bib-*` dependency direction remains valid. The new classic
runtime must not introduce dependencies from `bib-model`, `bib-input`, or
`bib-output` back toward `bib-engine`.

### 4.1 Proposed crate and module ownership

Add `bib-bst`, an internal semantic worker crate with no host I/O:

```text
crates/bib-bst/
  src/lib.rs                 public compile/execute boundary
  src/lexer.rs               byte-aware style tokenization
  src/parser.rs              top-level command and function parsing
  src/symbols.rs             typed symbols and declaration validation
  src/program.rs             immutable compiled representation
  src/compiler.rs            resolution and lowering
  src/value.rs               VM integers, strings, functions, and missing values
  src/vm.rs                  bounded stack and call execution
  src/builtins/              classic built-in families
  src/tests/                 internal parser, compiler, and VM tests
```

Keep job composition in `bib-engine`:

```text
crates/bib-engine/src/
  classic.rs                 classic jobs, options, attempts, and results
  classic/session.rs         VFS resource loop and content caches
  classic/aux.rs             recursive AUX parsing and control extraction
  classic/database.rs        classic database preparation and crossrefs
  classic/output.rs          transactional BBL/BLG sinks
  session.rs                 existing biblatex session, retained API
  bibliography.rs            backend-neutral enum facade
```

If `classic.rs` or any subordinate production file approaches the repository's
size target, split it by the ownership above rather than accumulating a single
interpreter module. Each new source area must receive a matching `AGENTS.md`
file-map update.

`bib-output` remains unchanged in responsibility. Classic output is generated
by VM effects because a style program controls every emitted byte.

### 4.2 Shared raw datasource boundary

The existing BibTeX datasource parser exposes a Biber-oriented source value,
but it eagerly decodes and recodes input, expands macros, collapses value-part
structure, and adds derived fields. It is not the classic boundary. Introduce
a genuinely lossless or compatibility-sufficient syntax layer beneath it,
while retaining the current eager source value as a Biber adapter, that
preserves:

- source order and byte locations;
- original and case-folded entry keys;
- entry type and field identifiers;
- concatenated value parts and string-macro references;
- bracing and control-sequence information needed by classic built-ins;
- `@string`, `@preamble`, and comment records; and
- duplicate and malformed-input recovery events.

Both backends may consume this representation, but conversion then branches:

```text
.bib bytes -> RawBibDatabase
                 |-- Biber conversion -> typed FieldValue model
                 `-- classic conversion -> VM-visible fields and preambles
```

Do not reuse Biber-normalized field values merely because the values originated
in BibTeX syntax. Classic built-ins observe brace structure, TeX control
sequences, case rules, missing fields, and string expansion differently.
Where the current parser has already discarded an observable distinction, add
a lossless layer rather than reconstructing it later.

## 5. Backend-neutral public facade

Retain `BibJob`, `BibSession`, `BibAttempt`, and `BibResult` as the stable
biblatex/Biber facade. Add a broader facade for callers that want backend
selection:

```rust
pub enum BibliographyJob {
    Biblatex(BibJob),
    Classic(ClassicBibJob),
}

pub struct ClassicBibJob {
    aux_path: VirtualPath,
    options: ClassicBibOptions,
}

pub enum BibliographyAttempt {
    Finished(BibliographyResult),
    NeedResources(NeedResources),
    Failed(BibliographyFailure),
}

pub enum BibliographyHistory {
    Spotless,
    Warning,
    Error,
    Fatal,
}

pub enum BibliographyDocument {
    Biblatex(Arc<ProcessedBibliography>),
    Classic(Arc<ClassicBibliography>),
}

pub struct BibliographyResult {
    backend: BibliographyBackend,
    history: BibliographyHistory,
    document: BibliographyDocument,
    files: Vec<GeneratedFile>,
    partial_files: Vec<GeneratedFile>,
    diagnostics: Vec<BibliographyDiagnostic>,
    stats: BibliographyStats,
}
```

`Finished` includes the reference program's spotless, warning, error, and
fatal histories. Warning and error histories may contain a complete `.bbl` and
remain eligible for project publication. Fatal history may retain detached
partial `.bbl` or `.blg` artifacts for command and parity inspection, but
project orchestration never publishes them. `Failed` is reserved for failures
outside the reference execution history, such as stale VFS state, an invalid
Umber configuration, or an internal invariant failure. The command adapter
maps history to the pinned WEB2C process status. This distinction is required
because classic BibTeX continues after many errors.

`BibliographySession` uses enum dispatch rather than a trait object at its
public boundary. This keeps initialization, cloning, equality, WASM exposure,
and exhaustiveness explicit:

```rust
pub enum BibliographySession {
    Biblatex(BibSession),
    Classic(ClassicBibSession),
}
```

Common diagnostics retain backend identity and a stable backend-specific code.
They do not flatten distinct concepts into a lowest-common-denominator error:

```rust
pub struct BibliographyDiagnostic {
    backend: BibliographyBackend,
    severity: Severity,
    code: BibliographyDiagnosticCode,
    message: String,
    source: Option<BibliographySourceLocation>,
}

pub enum BibliographyDiagnosticCode {
    Biblatex(BibDiagnosticCode),
    Classic(ClassicDiagnosticCode),
}
```

The classic document is a frozen audit/query value containing cited-entry
order, database records selected by `READ`, crossref inclusion, final sort
order, style identity, and execution summary. It must not expose mutable VM
stacks or symbol storage.

## 6. Mode selection

Backend selection is explicit in library and WASM APIs:

```rust
pub enum BibliographyMode {
    Biblatex { control_path: VirtualPath },
    Classic { aux_path: VirtualPath },
    Auto { job_path: VirtualPath },
}
```

Auto selection is owned by a backend-neutral `BibliographyDetector`, not by
`ClassicBibSession`. The detector parses enough of the root AUX closure to
recognize the classic protocol, can suspend with `NeedResources`, and returns
an immutable selected job plus a detection fingerprint. Its cache and
no-progress behavior follow the same VFS rules as bibliography sessions. A
no-bibliography result is distinct from incomplete classic control.

Explicit mode is recommended and has no heuristic behavior. Auto mode follows
a deterministic policy after a TeX pass:

1. If a generated `.bcf` exists and the ordered AUX closure contains classic
   `\bibstyle` or `\bibdata` commands, return an ambiguity diagnostic with the
   originating root or included AUX locations.
2. If a generated `.bcf` exists, select biblatex.
3. If the root AUX closure contains both `\bibstyle` and `\bibdata`, select
   classic.
4. If neither protocol is complete, do not run a bibliography backend.
5. If only one classic control command is present, report an incomplete classic
   configuration rather than guessing.

The CLI may expose `--bibliography=biblatex`, `--bibliography=classic`, and
`--bibliography=auto`. The existing `umber bib` command retains its current
Biber-compatible meaning until a separate classic invocation contract is
specified. Avoid changing an existing command based solely on input extension.

## 7. Auxiliary-file processing

The classic control stage parses the root AUX and recursive `\@input` closure.
It recognizes only the commands BibTeX owns:

- `\citation{...}`;
- `\bibdata{...}`;
- `\bibstyle{...}`; and
- `\@input{...}`.

All other TeX commands are ignored according to pinned reference behavior,
not executed as TeX. The parser must preserve encounter order, source file,
line, and byte location for diagnostics.

The AUX closure is resolved through `umber-vfs`. The pinned reference passes
the literal `\@input` name through its ordinary AUX file-opening/search path;
it does not automatically rebase it to the including AUX file's directory.
Umber must emulate the pinned working-directory and configured search order
with explicit VFS search areas, exact-extension behavior, and canonical
duplicate detection. Differential fixtures cover root, nested, user, and
distribution paths before any alternative localized resolution is adopted.
The session detects include cycles, duplicate control commands, missing
included files, and depth or byte limits.

The parsed control value contains:

```rust
pub struct ClassicControl {
    citations: Vec<CitationRequest>,
    databases: Vec<DatabaseRequest>,
    style: StyleRequest,
    aux_files: Vec<AuxIdentity>,
}
```

Citation `*`, duplicate citations, case-colliding keys, repeated `\bibdata`,
and repeated `\bibstyle` commands follow exact reference semantics. Do not
normalize these into Biber section or data-list values.

## 8. Style parsing and compilation

### 8.1 Input model

The style lexer operates on decoded compatibility characters while retaining
byte offsets. Classic mode initially supports the pinned reference's character
model. UTF-8 or 8-bit extensions require a distinct compatibility option and
fixture identity.

Tokens include identifiers, integer literals, quoted string literals,
function literals introduced by a quote, anonymous function bodies, braces,
and top-level command names. Comments and whitespace are discarded only after
their contribution to source locations is recorded.

### 8.2 Top-level commands

The parser supports all ten classic commands:

- `ENTRY`
- `EXECUTE`
- `FUNCTION`
- `INTEGERS`
- `ITERATE`
- `MACRO`
- `READ`
- `REVERSE`
- `SORT`
- `STRINGS`

Parsing is stateful because command order is semantic. The compiler records
whether `ENTRY` and `READ` have occurred and rejects repeated, premature, or
late commands with pinned recovery behavior. Recovery must make progress and
must be limited; it may scan to a blank line or another reference-defined
boundary, but never loop on the same token.

### 8.3 Symbols

Every symbol has one immutable declaration category:

```rust
pub enum SymbolKind {
    Builtin(Builtin),
    UserFunction(FunctionId),
    EntryField(FieldId),
    EntryInteger(EntryIntegerId),
    EntryString(EntryStringId),
    GlobalInteger(GlobalIntegerId),
    GlobalString(GlobalStringId),
    StringMacro(StringId),
    Special(SpecialSymbol),
}
```

The compiler implements reference-compatible case folding, identifier
characters, predefined symbols, duplicate checks, and shadowing prohibitions.
Symbol tables use deterministic maps and stable numeric IDs; map iteration must
not affect execution or diagnostics.

### 8.4 Program representation

Compile function bodies once instead of reparsing during execution:

```rust
pub struct CompiledStyle {
    compatibility: ClassicCompatibility,
    source: StyleSourceIdentity,
    declarations: Declarations,
    functions: Vec<CompiledFunction>,
    commands: Vec<CompiledCommand>,
    limits_charge: usize,
}

pub enum Instruction {
    PushInteger(i64),
    PushString(StringId),
    PushFunction(FunctionId),
    Call(FunctionId),
    Read(SymbolId),
    Assign(SymbolId),
    Builtin(Builtin),
}

pub enum CompiledCommand {
    Read,
    Execute(FunctionId),
    Iterate(FunctionId),
    Reverse(FunctionId),
    Sort,
}
```

Anonymous function literals compile to ordinary function IDs. Symbols follow
reference declaration-before-use behavior. Direct self-reference is diagnosed
as illegal recursion while scanning the function body, and forward references
are unknown; mutual recursion therefore cannot be constructed through ordinary
named functions. The compiler reproduces the reference's token omission and
recovery after these errors. Compiled programs contain no VFS handles, host
references, or mutable global state and may therefore be cached by compatibility
version plus content identity.

## 9. Style virtual machine

### 9.1 Values and storage

The runtime stack contains the value categories observable in classic BibTeX:

```rust
pub enum VmValue {
    Integer(i64),
    String(VmString),
    Function(FunctionId),
    Missing,
}
```

If the reference distinguishes a missing field from an empty string only at
specific built-ins, the internal representation must preserve that distinction
until the relevant coercion. Wrong-type stack values produce reference-style
diagnostics and recovery rather than Rust panics.

Storage is split into:

- immutable compiled constants and string macros;
- mutable global integer and string variables;
- per-entry integer and string variables;
- current-entry field views;
- the operand stack and call stack; and
- bounded output and log sinks.

Entry storage is allocated densely from declaration IDs. It is reset and
retained at the same lifecycle points as the reference implementation.

### 9.2 Execution lifecycle

The command stream drives execution:

1. Declarations and functions compile without a database.
2. `READ` loads cited entries and required crossrefs into classic storage.
3. `EXECUTE` runs once without a current entry.
4. `ITERATE` runs a function over current entry order.
5. `REVERSE` runs over reverse current order.
6. `SORT` stably reorders entries using `sort.key$` and pinned comparison
   semantics.
7. Later commands observe all permitted global and per-entry mutations.

The runtime must explicitly represent whether a current entry exists. Built-ins
that require an entry diagnose calls from `EXECUTE`; global-only operations do
not manufacture a dummy entry.

### 9.3 Built-ins

Implement built-ins in semantic families with focused tests:

- stack and control: `duplicate$`, `pop$`, `swap$`, `if$`, `while$`, `skip$`;
- arithmetic and comparison: `+`, `-`, `=`, `>`, `<`;
- variables and types: `:=`, `int.to.str$`, `int.to.chr$`, `chr.to.int$`,
  `type$`, `quote$`;
- strings: `*`, `substring$`, `text.length$`, `text.prefix$`, `purify$`,
  `change.case$`, `add.period$`, `empty$`;
- names: `num.names$`, `format.name$`;
- entry context: `cite$`, `call.type$`, `missing$`, `preamble$`;
- output and diagnostics: `write$`, `newline$`, `warning$`, `stack$`, `top$`;
- layout compatibility: `width$`; and
- any remaining predefined functions enumerated directly from the pinned
  source.

Before implementation begins, generate and commit an audited built-in census
from the pinned merged WEB plus change-file program. The census maps every
built-in to its source section, implementation owner, focused tests, and parity
fixtures. The default test gate rejects omissions and compatibility allowances.

String algorithms must operate on classic BibTeX text units, brace levels,
control sequences, and special-character groups rather than Rust Unicode scalar
or byte convenience APIs unless those exactly match the pinned mode.

### 9.4 Resource limits

`ClassicBibSessionOptions` defines validated hard bounds:

```rust
pub struct ClassicBibLimits {
    pub aux_bytes: usize,
    pub aux_files: usize,
    pub aux_depth: usize,
    pub database_bytes: usize,
    pub database_files: usize,
    pub fields_per_entry: usize,
    pub style_bytes: usize,
    pub style_tokens: usize,
    pub style_nesting: usize,
    pub symbols: usize,
    pub functions: usize,
    pub instructions: usize,
    pub entries: usize,
    pub stack_values: usize,
    pub call_depth: usize,
    pub string_bytes: usize,
    pub bbl_bytes: usize,
    pub blg_bytes: usize,
    pub terminal_bytes: usize,
    pub diagnostic_bytes: usize,
    pub retained_cache_bytes: usize,
    pub diagnostics: usize,
    pub work: usize,
}
```

The byte fields are aggregate job limits, not merely per-file limits. Retained
caches are bounded both by entry count and charged bytes, so a persistent WASM
session cannot retain several maximum-sized sources or compiled programs behind
a small entry-count bound. Style parsing uses an explicit nesting stack or a
small proven native-stack bound charged by `style_nesting`.

Every loop charges work, including lexer recovery, symbol lookup insertion,
database scanning, crossref closure, sorting comparisons, VM instruction
dispatch, name scanning, case conversion, width calculation, and output. Limit
failures are typed and deterministic. All arithmetic uses checked or explicitly
defined wrapping behavior matching the compatibility contract.

## 10. Classic database semantics

`READ` owns the semantic bridge from raw bibliography syntax to VM-visible
entries. It must implement:

- citation-order selection and `\citation{*}`;
- case-insensitive lookup with reference-compatible original-key retention;
- entry-type and field visibility declared by `ENTRY`;
- string-macro and month-macro expansion;
- preamble collection;
- duplicate entries and fields;
- crossref parent counting and `min_crossrefs` inclusion;
- crossref field inheritance and missing-parent diagnostics;
- entry order before style sorting; and
- missing versus empty field behavior.

These rules belong to the classic backend, not `bib-graph`, because their order
and observability are tied to `READ` and the VM. The implementation may reuse
pure helpers where equivalence is proven by fixtures.

## 11. Output and diagnostics

The VM writes to two detached, bounded sinks:

- a `.bbl` sink controlled by `write$` and `newline$`; and
- a `.blg`/terminal event sink controlled by engine diagnostics, `warning$`,
  tracing, and summary reporting.

The sinks track logical line length and reference wrapping rules separately
from stored bytes. Encoding is explicit. A suspended or infrastructure-failed
attempt exposes no generated files. A finished fatal reference history may
expose detached partial artifacts for inspection and invocation parity, but
marks them non-publishable. Project transactions discard those artifacts while
retaining typed diagnostics and the last accepted project output.

Diagnostics contain style/AUX/database source identity and byte/line context
where available. Rendering to `.blg` is a separate deterministic step over
typed events so library callers do not need to scrape text.

Exact compatibility fixtures compare:

- exit classification;
- `.bbl` bytes;
- `.blg` bytes after only explicitly documented environmental normalization;
- ordered typed diagnostics and source locations; and
- selected execution statistics where useful for limits.

## 12. VFS and caching

Add or reuse typed bibliography resource kinds for:

- root and included AUX files;
- BibTeX datasources;
- BST style programs; and
- optional compatibility resources in later extension modes.

This requires an explicit wire-level resource extension. Add stable AUX, BST,
and classic BIB kinds to `umber-vfs`, native and WASM request/result enums,
TypeScript declarations, browser resolvers, and distribution selection. Define
how the immutable distribution manifest classifies and shards style/data
resources. Native lookup documents whether it emulates `BIBINPUTS`/`BSTINPUTS`
or replaces them with ordered configured search areas. Each kind receives an
end-to-end injected-VFS, native resolver, distribution resolver, and browser
resolver test; unsupported resolvers must not silently report a required style
as generically unavailable.

The classic session follows the existing resumable protocol: a call either
finishes with a reference history, returns one deterministic batch of missing
resources, or fails outside reference execution.
Repeating the identical missing batch for the same job and snapshot is a typed
no-progress failure.

Bounded FIFO caches use keys containing every semantic input:

- parsed AUX: content identity, AUX compatibility version, and parse-limit
  identity;
- raw database: content identity, encoding, parser options, compatibility, and
  parse-limit identity;
- compiled style: content identity, decoding policy, compiler version,
  compatibility, and compile-limit identity; and
- optional prepared database: ordered datasource identities, control identity,
  a read-schema identity covering compiled `ENTRY` declarations and BST
  `MACRO` declarations, decoding, `min_crossrefs`, all read-affecting options,
  compatibility, and preparation limits.

Alternatively, a cache may store a canonical limit-independent representation
only if every hit deterministically recharges and revalidates it under the
current limits before returning it. A permissive earlier job must never let a
later restrictive job bypass byte, nesting, token, symbol, diagnostic, or work
limits.

Cached and cold attempts must return byte-identical files, diagnostics, and
observable statistics except explicitly identified cache counters.

## 13. Multipass project integration

Do not change the public `LatexProjectOptions.bibliography: BibJob` field in
place: existing downstream callers construct this public type with struct
literals, so a type change would be source-breaking. Preserve it as the
biblatex wrapper and add a versioned backend-neutral
`BibliographyProjectOptions`/`LatexProjectOptionsV2`, or deliberately schedule
a separately documented major API break. Compile-time downstream-usage tests
cover the retained legacy construction pattern as well as the new facade.

Each backend reports a control fingerprint:

- biblatex: `.bcf`, configuration, schemas, and datasource identities already
  used by `BibSession`;
- classic: the ordered AUX closure, `.bst`, ordered `.bib` identities,
  compatibility options, and relevant execution options.

The project loop becomes:

```text
begin pending VFS build
    -> run TeX
    -> inspect explicit or auto-selected bibliography protocol
    -> ask the selected backend to process its control closure
    -> return or accumulate resource needs discovered by the current stage
    -> publish detached .bbl/.blg into the pending generation
    -> rerun TeX
    -> converge on control, generated bibliography, and auxiliary identities
    -> atomically accept the project generation
```

The initial sequential implementation does not claim to discover TeX and
bibliography needs simultaneously: it completes or suspends the TeX stage,
then detection, then the selected bibliography stage. The public attempt may
merge needs already known from completed discovery steps, but it must not delay
a required response merely to speculate about a later stage. Detection,
backend processing, and project retry each have explicit no-progress state.

Switching backend mode between accepted revisions is a project configuration
change. It invalidates the prior bibliography fingerprint and must not reuse a
generated `.bbl` from the other backend. Oscillation detection includes backend
identity.

Classic mode should not rerun merely because unrelated AUX bytes changed if a
future semantic fingerprint is proven complete. Initial implementation uses
byte identity for the ordered AUX closure, favoring correctness over speculative
skips.

## 14. Native, WASM, and CLI APIs

The WASM project options gain a discriminated bibliography object while
accepting the existing biblatex shape for compatibility. JavaScript supplies
resources through the same generic request loop and never parses AUX, BST, or
BIB content.

The native host resolver maps style requests through the extended distribution
and VFS policy specified above. Semantic crates never call `kpsewhich` or
inspect `BSTINPUTS`/`BIBINPUTS`. The compatibility-mode `umber bibtex` adapter
must emulate the pinned ordered search contract before registering immutable
VFS resources. Any alternate configured-search mode receives a distinct
compatibility identity and is excluded from exact invocation and
file-resolution parity claims.

Add a classic command adapter only after its compatibility surface is defined.
Potential syntax is:

```text
umber bibtex [options] job.aux
```

It must call `ClassicBibSession` in-process, not spawn classic BibTeX. Command
exit status, default output names, terminal output, and `.blg` behavior receive
separate invocation fixtures.

## 15. Testing and fixtures

### 15.1 Hermetic default tier

All default tests consume committed fixtures and run through `cargo test
--tests`. They must not require a TeX Live executable, ICU, system locale,
network, or native filesystem search.

Test placement follows `docs/testing_policy.md`:

- `bib-bst/src/.../tests.rs` for lexer, parser, symbol, compiler, VM, built-in,
  limit, and adversarial internal tests;
- one `bib-engine/tests/it.rs` integration binary with classic submodules for
  public facade and parity tests;
- `umber` project-session tests for transactional multipass behavior; and
- shared reference fixtures under `tests/corpus/bibtex/`.

### 15.2 Fixture layout

```text
tests/corpus/bibtex/
  manifest.json
  upstream-0.99d/
    source-provenance.json
    web2c/
    bibtex-x/
  parser/
    valid/
    invalid/
  builtins/
  styles/
    plain/
    apalike/
    real-world/
  invocation/
  project/
```

The manifest pins the source commit; `bibtex.web`; `bibtex.ch`; generated
tangled/merged source identity; WEB2C configuration; build flags; applicable
kpathsea configuration; licenses; input identities; reference executable
identity; command options; expected status/history; and every generated file
identity.

### 15.3 Imported upstream tests

Import and isolate the TeX Live cases identified in the source audit:

- classic `bibtex.test` using `apalike.bst` and `xampl.bib`;
- classic memory styles `memdata1.bst` through `memdata3.bst`;
- classic AUX include, long-line, large-author, and output-path cases where
  their behavior belongs to the in-process boundary;
- BibTeX8/BibTeXu focused valid styles for built-ins whose classic behavior is
  shared; and
- `plain.bst` and `apalike.bst` whole-output runs.

The imported suite is a positive execution baseline, not sufficient parser
coverage. Preserve upstream source, expected outputs, and provenance so each
translation is auditable.

### 15.4 New parser suite

Add table-driven valid and invalid coverage for:

- every top-level command in every legal phase;
- repeated, missing, premature, and late commands;
- identifiers at character and length boundaries;
- nested and anonymous function bodies;
- quoted function literals;
- integers at numeric boundaries;
- strings with braces, quotes, percent comments, control sequences, and EOF;
- duplicate and cross-kind symbols;
- unknown functions and commands;
- malformed declaration lists;
- blank-line recovery and subsequent valid commands;
- exact line and byte source locations;
- token, nesting, symbol, instruction, diagnostic, and work limits; and
- arbitrary bytes under fuzz/property tests with guaranteed termination.

Each recovery test asserts both the diagnostic and the next successfully
recognized construct. Merely asserting that malformed input returns an error
does not validate recovery.

A systematic differential generator also enumerates bounded legal command
orders, declarations, function bodies, stack values, and database states. It
mutates every top-level command rule and built-in behavior branch identified by
the merged reference census. Coverage accounting records which reference
branches and state transitions have fixtures; completion thresholds are fixed
before declaring full-language compatibility. Real-world styles supplement
this systematic coverage rather than serving as proof of arbitrary-program
behavior.

### 15.5 VM and built-in suite

Each built-in receives focused tests covering:

- ordinary behavior;
- empty and missing values;
- every wrong stack type;
- operand stack underflow;
- brace/control-sequence edge cases;
- output and diagnostic order;
- compatibility character boundaries; and
- work and output limits.

Whole-style tests remain necessary because isolated built-ins do not validate
command lifecycle, entry context, mutation, sorting, or output composition.

### 15.6 Differential regeneration

`scripts/regen-fixtures.sh --area bibtex` is the only supported path that runs
the pinned reference executable or rewrites reference-derived fixtures. It:

1. builds or locates the pinned reference from an explicit source checkout;
2. verifies source and executable identity;
3. runs in a temporary isolated directory with fixed environment and no
   network;
4. captures status, terminal, `.bbl`, and `.blg` bytes;
5. applies only manifest-declared normalization;
6. atomically refreshes fixtures and their manifest; and
7. runs the hermetic Rust parity tests.

Ordinary Cargo tests never switch to a live reference through an environment
variable.

### 15.7 Real-world compatibility corpus

After standard styles pass, construct a redistribution-audited corpus from the
existing pinned arXiv sample and TeX Live styles. Select styles by language and
built-in coverage, not merely popularity. Record:

- style content identity and license;
- exercised commands and built-ins;
- datasource and citation characteristics;
- expected `.bbl` and diagnostics; and
- whether the case extends or merely duplicates existing coverage.

A census test rejects silent fixture loss and unowned compatibility gaps.

## 16. Performance and security

The style engine processes untrusted programmable input. It must be safe
against stack exhaustion, recursive calls, infinite `while$`, pathological
string concatenation, quadratic recovery, adversarial name formats, expensive
sort keys, and oversized output.

No VM function nesting uses the Rust call stack; ordinary user-function calls
and control primitives use an explicit bounded frame stack even though classic
recursive definitions are rejected. Parser nesting is likewise explicit or
bounded by a proven small stack limit. Strings use checked growth and a charged
representation.
Sorting charges comparisons and compared bytes. `while$` charges every
condition and body call. Diagnostic suppression after a limit still charges
work needed to reach a safe terminal state, or terminates immediately with a
typed limit failure.

Add explicit performance-tier benchmarks for:

- compiling `plain.bst` and `apalike.bst`;
- cold and cached execution over small and large databases;
- name formatting and case conversion;
- large field declarations;
- repeated concatenation and substring operations;
- sorting with long equal prefixes; and
- bounded rejection of malicious programs.

Persistent-session tests run many distinct maximum-charge jobs through one
WASM session and assert byte-weighted cache eviction and a bounded retained
allocation watermark.

Performance tests do not weaken correctness limits and do not run in ordinary
Cargo correctness tests unless they are fast deterministic regressions.

## 17. Implementation phases

### Phase 0: compatibility inventory and fixtures

- Pin the classic BibTeX source identity and executable build recipe.
- Produce command, built-in, diagnostic, and limit censuses from the merged
  `bibtex.web` plus `bibtex.ch` program and configured WEB2C build.
- Inventory the classic and `bibtex-x` test styles and expected outputs.
- Add the `tests/corpus/bibtex` manifest and regeneration mode.
- Translate the existing positive tests initially against the reference-only
  fixture harness.

Exit: every reference-owned construct has a named implementation/test owner,
and committed fixture regeneration is reproducible.

### Phase 1: backend-neutral facade

- Add backend-neutral job, session, attempt, result, history, diagnostic,
  partial-artifact, and document enums.
- Wrap existing `BibSession` without changing its behavior or public API.
- Adapt native and WASM result serialization to retain backend identity.
- Add facade tests proving old and wrapped biblatex paths are byte-identical.

Exit: all existing Biber tests pass unchanged and a no-op classic backend can
participate in typed resource/result plumbing.

### Phase 2: detection, AUX control, and resource loop

- Implement the backend-neutral resumable detector and bounded AUX parsing.
- Add typed AUX, BST, and classic datasource requests.
- Extend VFS wire kinds, native/WASM bindings, browser/native resolvers, and
  distribution selection for classic resources.
- Implement resource batching, conflicts, retry, and no-progress handling.
- Cache parsed AUX closures by content identities.
- Add invalid, cyclic, missing, duplicate, and path-resolution tests.

Exit: auto and explicit modes deterministically resolve their control/resource
closure through every host adapter without compiling or executing a style.

### Phase 3: lossless raw datasource boundary

- Audit observable information currently retained by `bib-input`.
- Introduce or extend the raw database representation.
- Preserve Biber conversion parity.
- Preserve ordered records, unexpanded value parts, macro declarations, brace
  information, and compatibility decoding needed by the later classic `READ`.
- Add raw parser observation fixtures while retaining the current eager
  `BibTexSource` as the Biber adapter.

Exit: both adapters consume the new raw boundary, all existing Biber parity is
unchanged, and no classic-observable syntax required by the census is lost.

### Phase 4: BST lexer, parser, and compiler

- Create `bib-bst` with source spans, limits, and typed diagnostics.
- Implement all top-level commands and phase validation.
- Implement typed symbol resolution and immutable compiled programs.
- Add the full negative parser/recovery suite.
- Add content-addressed compiled-style caching.

Exit: every committed valid style compiles, every invalid fixture matches
expected diagnostics/recovery, and arbitrary-byte tests terminate within
limits.

### Phase 5: classic `READ` and database preparation

- Consume the compiled `ENTRY` schema from Phase 4.
- Implement citation selection, macro expansion, preambles, duplicates,
  crossrefs, inheritance, declared-field projection, and entry order.
- Add focused reference-observation fixtures for VM-visible entry state.

Exit: classic entry storage matches reference-visible state for focused query
tests and is ready for command execution.

### Phase 6: VM core

- Implement explicit operand and call stacks.
- Implement variables, assignment, function calls, literals, and control-flow
  primitives.
- Implement command lifecycle, entry context, iteration, reverse iteration,
  and sorting.
- Add illegal-recursion rejection, nested-call, loop, mutation, wrong-type,
  underflow, and limit tests.

Exit: synthetic styles can read entries, mutate state, sort, and emit bounded
deterministic output without production panics.

### Phase 7: classic built-ins

- Implement built-ins by census family.
- Port focused BibTeX8/BibTeXu fixtures only where classic semantics agree.
- Add missing focused classic fixtures through regeneration.
- Complete name, text, case, purification, width, and diagnostic edge cases.

Exit: the built-in census has no unimplemented or untested classic entries and
focused parity passes.

### Phase 8: whole-style and command parity

- Pass `plain.bst` and `apalike.bst` whole-output fixtures.
- Match `.blg`, terminal, status, recovery, and invocation behavior.
- Add classic CLI adapter if approved by its command contract.
- Validate cold/cache identity and output limits.

Exit: pinned classic TeX Live tests and standard-style fixtures pass with no
compatibility allowances.

### Phase 9: project integration

- Add the versioned backend-neutral project API while preserving the legacy
  biblatex `LatexProjectOptions` contract.
- Add explicit and auto mode selection.
- Update native and WASM project options and result bindings.
- Add classic TeX-AUX-BibTeX-BBL-TeX convergence, suspension, rollback,
  oscillation, and backend-switch tests.

Exit: classic projects converge transactionally in native and WASM paths while
existing biblatex projects remain byte-identical.

### Phase 10: systematic and real-world hardening

- Add coverage-selected real-world styles and documents.
- Run the bounded legal-program differential generator and close the declared
  merged-reference coverage thresholds.
- Run explicit performance and malicious-input tiers.
- Tune dense storage and caches without changing observable behavior.
- Document supported compatibility identity and extension non-goals.

Exit: systematic reference-branch/state-transition thresholds and the selected
real-world corpus have exact reference parity, security limits terminate
adversarial cases, and performance budgets pass.

## 18. Commit and review strategy

Implement in logical commits that keep the default correctness gate green:

1. fixtures and censuses;
2. facade with unchanged biblatex behavior;
3. AUX/resource boundary;
4. raw database boundary;
5. parser/compiler;
6. classic `READ` and database preparation;
7. VM core;
8. built-in families;
9. standard-style parity;
10. versioned project/native/WASM integration; and
11. systematic and real-world hardening and documentation.

Refactors needed to preserve clear ownership land as separate preparatory
commits. Do not combine fixture regeneration with unrelated semantic changes.
Every phase closes its Beads issues only after its stated exit gate passes.

## 19. Principal risks and mitigations

### Lossy reuse of Biber parsing

Risk: classic behavior is reconstructed from normalized typed values and loses
brace, control-sequence, macro, or missing-field distinctions.

Mitigation: audit and expose a lossless raw database boundary before VM work;
run both Biber regression tests and focused classic observation fixtures at
that boundary.

### Treating positive execution as parser coverage

Risk: standard styles compile while malformed styles hang, overconsume memory,
or recover incompatibly.

Mitigation: build a table-driven negative/recovery suite and arbitrary-byte
termination tests before the VM is feature-complete.

### Unbounded programmable work

Risk: deeply nested calls, `while$`, string growth, name parsing, or sorting
causes denial of service, especially in WASM.

Mitigation: explicit stacks, pervasive work charging, checked growth, and hard
validated limits from the first executable VM commit.

### Facade leakage

Risk: backend-neutral APIs grow conditionals everywhere or erase useful
backend-specific data.

Mitigation: unify only attempts, resources, generated files, high-level
diagnostics, and orchestration. Preserve typed backend-specific documents,
options, diagnostic codes, and sessions behind enum variants.

### Project-mode ambiguity

Risk: a document emits both classic AUX control and a BCF, causing nondeterministic
or surprising backend choice.

Mitigation: explicit mode by default; auto mode reports ambiguity and includes
backend identity in convergence fingerprints.

### Reference fixture drift

Risk: live tools, host locale, path roots, or changing TeX distributions alter
expected output.

Mitigation: pinned source and executable identities, isolated regeneration,
manifested normalization, committed bytes, and hermetic Cargo tests.

### Completed-with-errors mismatch

Risk: treating every reference error as a failed attempt discards valid `.bbl`
output and makes status and diagnostic parity impossible.

Mitigation: model the four reference histories explicitly, retain detached
partial artifacts for fatal command inspection, and let project policy publish
only eligible finished results.

## 20. Exit criteria

The initial classic `.bst` mode is complete when:

- the pinned command and built-in censuses have no unowned entries;
- all ten top-level style commands parse and execute with positive and negative
  coverage;
- imported classic TeX Live tests pass through public Rust APIs;
- `plain.bst` and `apalike.bst` produce exact `.bbl`, `.blg`, diagnostics, and
  status for the committed corpus;
- the bounded differential generator meets declared merged-reference branch and
  state-transition thresholds, and selected real-world styles produce exact
  reference outputs;
- cold and cached execution are observably identical;
- malformed and malicious styles terminate under deterministic limits;
- native and WASM project sessions converge and roll back transactionally;
- existing biblatex/Biber APIs and pinned parity remain unchanged;
- `cargo test --tests`, `scripts/check.sh`, the WASM gate, and explicit
  bibliography performance gates pass; and
- the compatibility identity, limits, extension modes, fixture regeneration,
  and public APIs are documented.
