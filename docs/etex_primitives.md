# e-TeX V2 Extension Checklist

This checklist is deliberately separate from the TeX82 primitive inventory.
Compatibility mode installs only TeX82 meanings; `umber run --etex` selects
the extended primitive layer.

The behavioral contract is the [e-TeX manual](https://tex.org.uk/systems/doc/etex/etex_man.pdf), with the [short reference manual](https://mirror.gutenberg-asso.fr/tex.loria.fr/moteurs/etex_ref.html)
as its introductory companion.
The official `etex.ch` change file supplies implementation-level algorithms
where the manual does not specify them. Focused tests cite the relevant manual
section and compare observable behavior with e-TeX/pdfTeX. The official
two-phase e-TRIP gate additionally binds the exact e-TeX 2.6 semantic/text/DVI
oracle to the pinned V2 terminal, log, DVItype, and generated-output masters;
its narrow normalization and profile-adaptation contract is documented in
[TRIP](trip.md).

Status values are **done**, **partial**, and **missing**. A family is done only
after its focused parity fixtures and compatibility-mode visibility checks
pass.

## Expansion and virtual input (manual sections 3.1, 3.2, 3.6, 3.7)

| Primitive            | Status | Manual contract / remaining gate                                                                                                                                                                                                                      |
| -------------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `\protected`         | done   | Ordinary expansion expands the macro; `\edef`, `\write`, alignment fetches, and analogous expanded-token-list contexts preserve it.                                                                                                                   |
| `\unexpanded`        | done   | Yields the raw balanced text; expanded-token-list builders copy that result without further expansion, while ordinary `get_x_token` processing expands the returned tokens normally. Its compulsory opener uses TeX's `scan_left_brace` recovery.     |
| `\detokenize`        | done   | Produces only catcode-10 spaces and catcode-12 other characters; every control word produces a trailing space, including the last.                                                                                                                    |
| `\readline`          | done   | Reads through the virtualized `\read` path with catcode-10 codepoint 32 and catcode-12 other characters, including `\endlinechar`.                                                                                                                    |
| `\scantokens`        | done   | Serializes unexpanded general text with TeX's `new_string` character behavior, splits `\newlinechar` into pseudo-file records, and reprocesses under current catcodes and `^^` notation.                                                              |
| `\everyeof`          | done   | Inserts its tokens once at natural EOF for real and generated virtual files, but not for `\endinput`, and remains ordered before the pseudo-file closing trace. Its grouped, snapshot-covered token parameter is distinct from TeX's `\errhelp` cell. |
| `\unless`            | done   | Negates every boolean conditional through the shared conditional-frame path and rejects `\ifcase` as the manual requires.                                                                                                                             |
| `\tracingscantokens` | done   | Positive values trace `(␣` at pseudo-file entry and `)` only after any `\everyeof` replay, as specified in section 3.6.                                                                                                                               |

The committed `etex_exec/expansion_virtual_input` reference fixture covers
the observable family against pdfTeX/e-TeX. Focused tests additionally cover
protected expansion contexts, invalid `\unless`, `\endinput`, and restoration
of a live pseudo-file from its input summary with identical replay output and
aggregate state hash. Compatibility-mode visibility is checked independently
for every primitive and parameter in the family.

## Environmental and conditional enquiries (manual section 3.3)

`\eTeXversion`, `\eTeXrevision`, `\ifdefined`, non-creating `\ifcsname`, and
`\iffontchar` are implemented with focused V2 tests. The later live-name-scan
enquiry `\ifincsname` is a pdfTeX 1.40.29 primitive and remains undefined in
the e-TeX 2.6 profile. `\currentgrouplevel`,
`\currentgrouptype`, `\currentiflevel`, `\currentiftype`, and
`\currentifbranch` read exact resumable group/conditional state.
`\lastnodetype` is implemented from `etex.ch` [26.424]'s effective tail:
inner-mode enquiries read the live current list, including an unmaterialized
horizontal character run, while outer vertical mode follows the contribution
tail and page-builder memo. Empty lists return -1 and other tails use the
manual/e-TRIP node codes. `\iffontchar` reads the same immutable metrics as
typesetting and the font dimension enquiries. Malformed font selectors use
TeX's `back_error` recovery: they diagnose the missing identifier, substitute
the null font, and leave the offending token for the following number scanner.

## Expressions and value enquiries (manual section 3.5)

`\numexpr` is implemented with manual-defined precedence, parentheses,
rounded division, combined multiply/divide, and overflow recovery. `\dimexpr`
implements the same grammar with dimension-first terms and exact scaled-point
rounding. `\glueexpr` and `\muexpr` implement the same grammar componentwise,
including dominant infinite orders and combined scaling. `\gluestretch`,
`\glueshrink`, `\gluestretchorder`, and `\glueshrinkorder` expose the manual's
component values and order codes; `\gluetomu` and `\mutoglue` preserve all
components and their e-TeX [53a.5404--5425] glue-pointer identity while
changing the unit type. Local `\skip` and `\muskip` writes therefore take
e-TeX [19.277]'s `reassigning` branch after a no-op conversion/expression
round trip instead of comparing only their rendered components. `\fontcharwd`,
`\fontcharht`, `\fontchardp`, and `\fontcharic` are implemented as read-only
internal dimensions.

## Diagnostics and mutable state (manual sections 3.4, 3.6)

`\interactionmode` is implemented as a globally assigned read/write view of
the checkpointed interaction state. `\showtokens` uses `etex.ch`
[17.3623--3671]'s command-owned, unexpanded balanced general-text scan, removes
the compulsory braces, and emits a detached token-bearing diagnostic effect
before the ordinary terminal/transcript rendering. `\showgroups` walks the
live checkpointed group stack. `\showifs` uses `etex.ch` [17.3703--3732]'s
innermost-first traversal of a typed detached conditional snapshot, including
`\unless`, `\else`, and saved source-line rendering, without mutating command
state. `\tracingassigns`, `\tracinggroups`, `\tracingifs`, and
`\tracingnesting` are implemented as group-scoped integer parameters, and
three of the four render `etex.ch`'s exact rendered trace text:

The pinned TeX Live Web2C [54/SyncTeX] layer also installs `\synctex` as a
group-scoped integer parameter in the extended engine profile. Its default is
zero, assignments and format persistence use the ordinary integer-parameter
path, and TeX82 compatibility mode omits it because that oracle's change stack
does not apply the SyncTeX layer.

- `\tracinggroups` renders `etex.ch` [19.274/19.281]'s `group_trace`
  `{entering ...}`/`{leaving ...}` lines from `Universe::enter_group_with_kind_at_line`/
  `leave_group_with_kind` (`crates/tex-state/src/etex_tracing.rs`), so every
  group-open/close call site is covered uniformly.
- `\tracingassigns` renders `etex.ch` [17.687-750]'s `assign_trace`/
  `restore_trace` `{into ...}`/`{reassigning ...}`/`{changing ...}`/
  `{globally changing ...}` lines (`crates/tex-exec/src/assignments/tracing.rs`),
  hooked at `main_control.rs`'s `apply_scanned_step` for the
  integer/dimension/glue/mu-glue/token register and parameter families, the
  six code tables, and `\def`/`\edef`/`\gdef`/`\xdef`/`\let`/`\futurelet`
  meaning assignments. `\setbox`, `\font`/`\textfont`-family font selection,
  and page-builder scalars (not eqtb-resident in real TeX82, so page
  integers/dimensions are correctly untraced) remain open in `umber2-38hs`.
- `\tracingifs` renders `etex.ch` [28.498/28.494/28.510]'s extra
  `show_cur_cmd_chr` calls at conditional entry, at an ordinarily-arriving
  `\or`/`\else`/`\fi`, and at one found while `pass_text` skips unselected
  material (`crates/tex-command/src/conditionals.rs`), including the
  `\unless` prefix and `(level N) entered on line L` suffix. It does not yet
  render `show_cur_cmd_chr`'s own mode-change prefix (`umber2-wb0m`), since
  that state is owned by the executor's mode nest, a layer the command core
  does not reach.

`\tracingnesting` renders `etex.ch` [23.328]'s `file_warning`: a source level
receives the live group and conditional ancestry in its canonical opening
transition (`open_registered_input`), and its retirement
(`crates/tex-command/src/processor/next.rs`'s `retire_input_top`) compares
the moved frame-owned recording against the live depth, printing a "Warning: end of file when
... is incomplete" line for each group and conditional still open
(`crates/tex-command/src/tracing_nesting.rs`). Unlike the other
three parameters, this prints through the ambient selector rather than
`begin_diagnostic`'s `\tracingonline` redirect, matching `etex.ch`'s own
`file_warning`, which is not `stat`-gated. The sibling `if_warning` path and
ordinary/semi-simple `group_warning` closes compare against the same source
opening depths; `\scantokens` pseudo-files now record those depths too.
Specialized group closures remain tracked by `umber2-aqx9`, since their
`leave_group_with_kind` sites have no single choke point analogous to
`file_warning`'s.

## Marks, lists, paragraph extensions, and math (manual sections 3.4, 3.7)

The `umber2-wvo.4` state and paragraph family is **done** against the manual
contract and the official e-TRIP workload:

- Section 3.4's 16-bit register range is covered for `\count`, `\dimen`,
  `\skip`, `\muskip`, and `\toks`, including local restoration at indexes 256
  and 32767. As required by `etex.ch`, compatibility mode retains TeX82's
  0..255 register limit and leaves the extension-only control sequences
  undefined.
- The mark-class family tracks independent top/first/bottom values through
  page fire-up and independent split-first/split-bottom values through
  `\vsplit`. Canonical `\marks` follows `etex.ch` [26.424]: it scans a
  `0..=32767` class with recover-to-zero diagnostics, expands its balanced
  mark text, and appends the selected class in every mode; TeX82 retains only
  class-zero `\mark`. `\pagediscards` and `\splitdiscards` destructively
  splice the lists retained when `\savingvdiscards` is positive.
- All four penalty arrays implement manual-defined assignment, repeated final
  entries, grouping, and forward/reverse line indexing. The interline array is
  reset at paragraph completion as specified.
- `\parshapelength`, `\parshapeindent`, and `\parshapedimen` expose explicit
  and repeated shape components. `\lastlinefit` follows the `etex.ch`
  line-adjustment algorithm and all fifteen official e-TRIP outcomes.
  `\savinghyphcodes` snapshots per-language lowercase mappings for later
  pattern and exception use.

TeX--XeT directions and `\middle` are implemented. Direction nodes survive box
packing and snapshots, nested right-to-left and left-to-right segments are
resolved into ordinary DVI visual order at shipout, open direction segments
are closed and resumed across broken paragraph lines, and display interruption
sets `\predisplaydirection` before resuming the active segments. Display math
and equation numbers remain left-to-right as required by manual section 4.1.
`\middle` shares the enclosing `\left...\right` delimiter extent and uses a
right-boundary class on its left and a left-boundary class on its right, so it
does not accidentally acquire relation glue.

## Conformance gates

The optional in-process two-phase `e2e_conformance_etrip` test passes exact DVI
comparison against the locally generated pdfTeX/e-TeX oracle when its external
inputs are installed. The always-available focused tests cover every primitive
family and the compatibility-mode visibility boundary.

- Compatibility mode: every extension control sequence remains undefined and
  unused extended mode retains TeX82 Story/Gentle/TRIP behavior.
- Focused corpus: exact expansion, diagnostics, state, node-list, and DVI
  parity for every family above. Fixture regeneration uses only
  `scripts/regen-fixtures.sh`.
- Diagnostic trace wording for `\tracingassigns`, `\tracinggroups`, and
  `\tracingifs` is pinned by
  `crates/tex-exec/src/main_control/etex_diagnostic_tracing.rs`'s
  focused fixtures, and `\tracingnesting`'s `file_warning` case by
  `crates/tex-command/src/processor/expand/tests.rs`'s, against real
  e-TeX/pdfTeX 1.40.25 output captured with each parameter set in isolation.
  `\tracingnesting`'s `group_warning`/`if_warning` case, `\tracingassigns`
  coverage of box registers and font selection, and `\tracingifs`'s
  mode-change prefix remain open (`umber2-aqx9`, `umber2-38hs`,
  `umber2-wb0m`).
