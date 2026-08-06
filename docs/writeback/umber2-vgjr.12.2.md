# `umber2-vgjr.12.2` writeback

Command-semantic now has one fixture-local V2 manifest per case. Directory structure infers domain, ID, and source; expected-file presence infers ordinary file-or-empty channels; clean status, pass expectation, and profile capture default structurally. The sole capture exception is explicit on `main-control/hyphenation-data`.

All 203 resolved cases are pinned by one digest covering identity, profile/route, capture, provenance, projections, 1,233 explicit expected strings, xfails, channels, statuses, inputs, and interaction policy. The generated schema is byte-compared with the Rust type, and the Git-backed closed-inventory gate remains green.

The detached 173-line capture catalogue and 467-line duplicate census were deleted. Manifest data fell from 10,448 to 7,258 lines (3,190 lines removed); authored Rust/shell changed by +282/-832 lines (net -550), while the corpus/schema/catalogue changed by +7,277/-10,396 lines (net -3,119).

Focused `tex-command-stream` tests passed under the 512 MiB/1 GiB limits. The complete native workspace suite passed under 1 GiB, and `scripts/check.sh` reported all four gates passed under 1 GiB.
