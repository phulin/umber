# e-TeX V2 Extension Checklist

This checklist is deliberately separate from the TeX82 primitive inventory.
Compatibility mode installs only TeX82 meanings; `umber run --etex` selects
the extended primitive layer.

The behavioral contract is the [e-TeX
manual](https://tex.org.uk/systems/doc/etex/etex_man.pdf), with the [short
reference manual](https://mirror.gutenberg-asso.fr/tex.loria.fr/moteurs/etex_ref.html)
as its introductory companion.
The official `etex.ch` change file supplies implementation-level algorithms
where the manual does not specify them. Focused tests must cite the relevant
manual section and compare observable behavior with e-TeX/pdfTeX. The final
gate is the official e-TRIP suite, not implementation similarity.

Status values are **done**, **partial**, and **missing**. A family is done only
after its focused parity fixtures and compatibility-mode visibility checks
pass.

## Expansion and virtual input (manual sections 3.1, 3.2, 3.6, 3.7)

| Primitive | Status | Manual contract / remaining gate |
| --- | --- | --- |
| `\protected` | done | Ordinary command demand expands the macro; `\edef`, `\write`, alignment fetches, and analogous expansion-only contexts preserve it. |
| `\unexpanded` | done | Yields the raw balanced text as token-list expansion does: expansion-only consumers preserve it, while later command demand expands it. |
| `\detokenize` | done | Produces only catcode-10 spaces and catcode-12 other characters; every control word produces a trailing space, including the last. |
| `\readline` | done | Reads through the virtualized `\read` path with catcode-10 codepoint 32 and catcode-12 other characters, including `\endlinechar`. |
| `\scantokens` | done | Serializes unexpanded general text with TeX's `new_string` character behavior, splits `\newlinechar` into pseudo-file records, and reprocesses under current catcodes and `^^` notation. |
| `\everyeof` | done | Inserts its tokens once at natural EOF for real and generated virtual files, but not for `\endinput`, and remains ordered before the pseudo-file closing trace. |
| `\unless` | done | Negates every boolean conditional through the shared conditional-frame path and rejects `\ifcase` as the manual requires. |
| `\tracingscantokens` | done | Positive values trace `( ` at pseudo-file entry and `)` only after any `\everyeof` replay, as specified in section 3.6. |

The committed `etex_exec/expansion_virtual_input` reference fixture covers
the observable family against pdfTeX/e-TeX. Focused tests additionally cover
protected expansion contexts, invalid `\unless`, `\endinput`, and restoration
of a live pseudo-file from its input summary with identical replay output and
aggregate state hash. Compatibility-mode visibility is checked independently
for every primitive and parameter in the family.

## Environmental and conditional enquiries (manual section 3.3)

`\eTeXversion`, `\eTeXrevision`, `\ifdefined`, and non-creating `\ifcsname`
are implemented with focused V2 tests. `\currentgrouplevel`,
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
are installed as group-scoped integer parameters, but their trace emission is
**missing**. The remaining work is observable diagnostic behavior rather than
primitive registration or assignment scanning.

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

TeX--XeT directions and `\middle` are implemented with focused tests but remain
outside this completed state-family audit. The two-phase e-TRIP DVI gate passes,
but the dedicated direction/math task still owns focused DVI fixtures for
nested directions, boxed direction nodes, display direction, equation-number
placement, and `\left...\middle...\right` layout.

## Conformance gates

The in-process two-phase `e2e_conformance_etrip` test currently passes exact
DVI comparison against the locally generated pdfTeX/e-TeX oracle after the
documented preamble-comment normalization. This completes the current e-TRIP
DVI fixture gate; it does not complete the official textual-artifact gate.

- Compatibility mode: every extension control sequence remains undefined and
  unused extended mode retains TeX82 Story/Gentle/TRIP behavior.
- Focused corpus: exact expansion, diagnostics, state, node-list, and DVI
  parity for every family above. Fixture regeneration uses only
  `scripts/regen-fixtures.sh`.
- Official e-TRIP remaining work: exact parity-mode log, terminal-photo,
  DVItype, and output-file comparison from pinned inputs; deliberate text and
  DVI perturbations must fail actionably.
