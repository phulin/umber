# umber2-e51h.68.4 — extension dispatch and base whatsits

TeX82 §§1340–1361 define the six base extension selectors, their any-mode
dispatch, stream normalization, and the five base whatsit subtypes. Umber's
canonical command core owns scanning and mode dispatch; `tex-exec` owns typed
node construction and diagnostic display.

`extension_commands_dispatch_in_each_mode_with_canonical_stream_fallbacks`
exercises all six semantic modes, §1350's distinct openout and write/close
stream boundaries, immediate dispatch, and §1377's branch-local setlanguage
legality. Existing focused scanner tests retain the exact slot assertions:
openout uses the four-bit recovery while write and closeout map positive and
negative out-of-range values to the permanently closed 16 and 17 cases.

`whatsit_copy_free_display_and_default_output_name_are_subtype_complete`
covers §§1349–1361's open, write, close, special, and language variants. It
proves that an extensionless `\openout` name stays extensionless, an explicit
extension stays intact, write text remains an owned unexpanded token list,
special text is fixed after expanded scanning, cloned nodes remain valid after
the copy is dropped, and every base subtype has TeX-style diagnostic output.
The typed Rust values replace WEB's word-size-specific allocation/free paths;
clone/drop and zero-dimension list tests are their observable ownership and
packing contract.
