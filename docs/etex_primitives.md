# e-TeX V2 Extension Checklist

This checklist is deliberately separate from the TeX82 primitive inventory.
Compatibility mode installs only TeX82 meanings; `umber run --etex` selects
the extended primitive layer.

The behavioral contract is the [e-TeX short reference
manual](https://mirror.gutenberg-asso.fr/tex.loria.fr/moteurs/etex_ref.html).
The official `etex.ch` change file supplies implementation-level algorithms
where the manual does not specify them. Focused tests must cite the relevant
manual section and compare observable behavior with e-TeX/pdfTeX. The final
gate is the official e-TRIP suite, not implementation similarity.

Status values are **done**, **partial**, and **missing**. A family is done only
after its focused parity fixtures and compatibility-mode visibility checks
pass.

## Expansion and virtual input (manual sections 3.1, 3.2, 3.7)

| Primitive | Status | Manual contract / remaining gate |
| --- | --- | --- |
| `\protected` | partial | Ordinary command demand expands the macro; `\edef`, `\write`, and analogous expansion-only contexts preserve it. Alignment-specific protected fetching and reference parity remain. |
| `\unexpanded` | partial | Yields the raw balanced text as token-list expansion does: expansion-only consumers preserve it, while later command demand expands it. Reference parity remains. |
| `\detokenize` | partial | Produces only catcode-10 spaces and catcode-12 other characters; every control word produces a trailing space, including the last. Reference parity remains. |
| `\readline` | partial | Reads through the virtualized `\read` path with catcode-10 codepoint 32 and catcode-12 other characters, including `\endlinechar`; normalized transcript parity remains. |
| `\scantokens` | missing | Serializes unexpanded general text as a pseudo-file, then reprocesses it through the input mechanism under current catcodes, including `^^` notation. |
| `\everyeof` | missing | Inserts its tokens once at natural EOF for real and virtual files, but not for `\endinput`. |
| `\unless` | partial | Negates the shared boolean-conditional evaluation path without adding pending input state; focused reference-error parity remains. |
| `\tracingscantokens` | missing | Positive values trace pseudo-file open and close as specified in section 3.6. |

## Environmental and conditional enquiries (manual section 3.3)

`\eTeXversion`, `\eTeXrevision`, `\currentgrouplevel`,
`\currentgrouptype`, `\currentiflevel`, `\currentiftype`,
`\currentifbranch`, `\ifdefined`, `\ifcsname`, `\iffontchar`, and
`\lastnodetype` are **missing**. In particular, `\ifcsname` must neither
create a hash-table entry nor assign `\relax` to a missing name.

## Expressions and value enquiries (manual section 3.5)

`\numexpr`, `\dimexpr`, `\glueexpr`, `\muexpr`, `\gluestretch`,
`\glueshrink`, `\gluestretchorder`, `\glueshrinkorder`, `\gluetomu`,
`\mutoglue`, `\fontcharwd`, `\fontcharht`, `\fontchardp`, and
`\fontcharic` are **missing**.

## Diagnostics and mutable state (manual sections 3.4, 3.6)

`\interactionmode`, `\showgroups`, `\showifs`, `\showtokens`,
`\tracingassigns`, `\tracinggroups`, `\tracingifs`, and `\tracingnesting`
are **missing**.

## Marks, lists, paragraph extensions, and math (manual sections 3.4, 3.7)

The mark-class family, discard-list enquiries, penalty arrays, parshape
enquiries, `\lastlinefit`, `\savinghyphcodes`, `\savingvdiscards`, TeX--XeT
direction family, and `\middle` are **missing**. Sparse register support for
indexes 256 through 32767 and enhanced `\parshape` parsing still require an
explicit e-TeX audit even where the state substrate already supports them.

## Conformance gates

- Compatibility mode: every extension control sequence remains undefined and
  unused extended mode retains TeX82 Story/Gentle/TRIP behavior.
- Focused corpus: exact expansion, diagnostics, state, node-list, and DVI
  parity for every family above. Fixture regeneration uses only
  `scripts/regen-fixtures.sh`.
- Official e-TRIP: pinned inputs and exact parity-mode log/photo/output plus
  DVI/DVItype comparison; deliberate text and DVI perturbations must fail.
