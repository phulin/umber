# Structured scanner ownership

The command processor is the sole live owner of structured TeX scans. A scan
borrows its input, command state, attempt arena, and diagnostic channel for one
synchronous operation. A resource request unwinds the call; the host restores
the full checkpoint before retry. No family module stores an independent
scanner status, error queue, or replay cursor.

The structured scanner is organized by the result being prepared for the
executor:

| Family                       | Responsibility                                                                                  |
| ---------------------------- | ----------------------------------------------------------------------------------------------- |
| Definitions and general text | Definition targets, balanced text, macro and `\let` operands, and case conversion.              |
| PDF                          | Object, form, image, graphics, navigation, action, and document-fragment requests.              |
| Math                         | Field classification, delimiter and fraction operands, family, and math-material requests.      |
| Box and alignment            | Box specifications and payloads; alignment preamble scanning and its live scanner episode.      |
| Input and output             | Filename and stream operations, immediate extensions, write expansion, and display diagnostics. |

The modules add no new scanner framework. They implement methods on the
existing `CommandProcessor` and use the same scalar scanner, token collector,
delivery, and recovery methods. Result and local-progress declarations live in
private value modules, with explicit reexports preserving the original
`structured` namespace and crate API. Executor callers receive
completed, typed operands and never resume a partial source scan. Tests that
guard source-level delivery and scanner-status boundaries inspect every
structured family implementation; fixture tests verify token consumption and
error ordering.
