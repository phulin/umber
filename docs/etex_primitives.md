# e-TeX V2 Extension Checklist

This checklist is deliberately separate from the TeX82 primitive inventory.
Compatibility mode installs only TeX82 meanings; `umber run --etex` selects
the extended primitive layer.

The behavioral contract is the [e-TeX manual](https://tex.org.uk/systems/doc/etex/etex_man.pdf), with the [short reference manual](https://mirror.gutenberg-asso.fr/tex.loria.fr/moteurs/etex_ref.html)
as its introductory companion.
The official `etex.ch` change file supplies implementation-level algorithms
where the manual does not specify them. Focused tests cite the relevant manual
section and compare observable behavior with e-TeX/pdfTeX. Per the 2026-07-14
project scope decision, the repository's focused tests and existing in-process
two-phase e-TRIP DVI fixture are the conformance gates; a separate official
text-artifact harness is not planned.

Status values are **done**, **partial**, and **missing**. A family is done only
after its focused parity fixtures and compatibility-mode visibility checks
pass.

## Expansion and virtual input (manual sections 3.1, 3.2, 3.6, 3.7)

| Primitive            | Status | Manual contract / remaining gate                                                                                                                                                                                                                      |
| -------------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `\protected`         | done   | Ordinary expansion expands the macro; `\edef`, `\write`, alignment fetches, and analogous expanded-token-list contexts preserve it.                                                                                                                   |
| `\unexpanded`        | done   | Yields the raw balanced text; expanded-token-list builders copy that result without further expansion, while ordinary `get_x_token` processing expands the returned tokens normally.                                                                  |
| `\detokenize`        | done   | Produces only catcode-10 spaces and catcode-12 other characters; every control word produces a trailing space, including the last.                                                                                                                    |
| `\readline`          | done   | Reads through the virtualized `\read` path with catcode-10 codepoint 32 and catcode-12 other characters, including `\endlinechar`.                                                                                                                    |
| `\scantokens`        | done   | Serializes unexpanded general text with TeX's `new_string` character behavior, splits `\newlinechar` into pseudo-file records, and reprocesses under current catcodes and `^^` notation.                                                              |
| `\everyeof`          | done   | Inserts its tokens once at natural EOF for real and generated virtual files, but not for `\endinput`, and remains ordered before the pseudo-file closing trace. Its grouped, snapshot-covered token parameter is distinct from TeX's `\errhelp` cell. |
| `\unless`            | done   | Negates every boolean conditional through the shared conditional-frame path and rejects `\ifcase` as the manual requires.                                                                                                                             |
| `\tracingscantokens` | done   | Positive values trace `(` at pseudo-file entry and `)` only after any `\everyeof` replay, as specified in section 3.6.                                                                                                                                |

The committed `etex_exec/expansion_virtual_input` reference fixture covers
the observable family against pdfTeX/e-TeX. Focused tests additionally cover
protected expansion contexts, invalid `\unless`, `\endinput`, and restoration
of a live pseudo-file from its input summary with identical replay output and
aggregate state hash. Compatibility-mode visibility is checked independently
for every primitive and parameter in the family.

## Environmental and conditional enquiries (manual section 3.3)

`\eTeXversion`, `\eTeXrevision`, `\ifdefined`, non-creating `\ifcsname`, and
live-name-scan enquiry `\ifincsname` are implemented with focused V2 tests. `\currentgrouplevel`,
`\currentgrouptype`, `\currentiflevel`, `\currentiftype`, and
`\currentifbranch` read exact resumable group/conditional state.
`\lastnodetype` is implemented from the effective current-list/page tail with
the manual/e-TRIP node codes. `\iffontchar` reads the same immutable metrics
as typesetting and the font dimension enquiries.

## Expressions and value enquiries (manual section 3.5)

`\numexpr` is implemented with manual-defined precedence, parentheses,
rounded division, combined multiply/divide, and overflow recovery. `\dimexpr`
implements the same grammar with dimension-first terms and exact scaled-point
rounding. `\glueexpr` and `\muexpr` implement the same grammar componentwise,
including dominant infinite orders and combined scaling. `\gluestretch`,
`\glueshrink`, `\gluestretchorder`, and `\glueshrinkorder` expose the manual's
component values and order codes; `\gluetomu` and `\mutoglue` preserve all
components while changing the unit type. `\fontcharwd`, `\fontcharht`, `\fontchardp`, and
`\fontcharic` are implemented as read-only internal dimensions.

## Diagnostics and mutable state (manual sections 3.4, 3.6)

`\interactionmode` is implemented as a globally assigned read/write view of
the checkpointed interaction state. `\showtokens` displays the manual-defined
decomposition of unexpanded balanced text. `\showgroups` and `\showifs` walk
the live checkpointed group and conditional stacks. `\tracingassigns`,
`\tracinggroups`, `\tracingifs`, and `\tracingnesting`
are implemented as group-scoped integer parameters. Exact diagnostic trace
text parity for those four parameters is explicitly deferred; it is not part
of the primitive-completeness gate.

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
  `\vsplit`. `\pagediscards` and `\splitdiscards` destructively splice the
  lists retained when `\savingvdiscards` is positive.
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
- Diagnostic trace wording for `\tracingassigns`, `\tracinggroups`,
  `\tracingifs`, and `\tracingnesting` is deferred by scope.
