//! tex.web §§660-675's overfull/underfull box diagnostics.
//!
//! `hpack` and `vpack` compute a badness and, when it exceeds `\hbadness` or
//! `\vbadness` (or the box overflows past `\hfuzz`/`\vfuzz`), report it. The
//! report is the same shape in both directions:
//!
//! ```text
//! Underfull \hbox (badness 10000) detected at line 13
//!    <short_display of the hlist, hbox only>
//! <show_box of the packed box, transcript only>
//! ```
//!
//! [`tex_typeset`] already decides *whether* a box is bad enough to report --
//! that is a pure function of the glue setting and the badness thresholds --
//! and returns [`PackDiagnostic`]s saying so. This module owns everything the
//! decision does not: which of `Underfull`/`Loose` (§663) or `Tight` (§664)
//! names a stretching or shrinking box, where the report says the box came
//! from (§661's `pack_begin_line`, §660's `line`), and the abbreviated list
//! display §174's `short_display` produces.
//!
//! §245's `begin_diagnostic` carries the `show_box` half to the transcript
//! alone unless `\tracingonline` is positive, and raises `history` to
//! `warning_issued` on the way, which is what makes §1335 tell the terminal
//! the transcript has more.

#[cfg(test)]
mod tests;

use std::fmt::Write as _;

use tex_state::Universe;
use tex_state::ids::NodeListId;
use tex_state::node::Node;
use tex_typeset::PackDiagnostic;

use crate::node_dump::{DumpConfig, dump_node_slice};

/// Which of §660's and §674's two reporting sites is speaking.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PackedDirection {
    /// §660's `hpack`: reports `\hbox`, widths, and §174's `short_display`.
    Horizontal,
    /// §674's `vpack`: reports `\vbox` and heights, and has no
    /// `short_display` -- an abbreviated display of a vertical list would say
    /// nothing.
    Vertical,
}

impl PackedDirection {
    const fn box_name(self) -> &'static str {
        match self {
            Self::Horizontal => " \\hbox (",
            Self::Vertical => " \\vbox (",
        }
    }

    /// §660's `pt too wide` against §674's `pt too high`.
    const fn overfull_excess_text(self) -> &'static str {
        match self {
            Self::Horizontal => "pt too wide",
            Self::Vertical => "pt too high",
        }
    }
}

/// Reports every diagnostic one `hpack`/`vpack` produced.
///
/// `packed` is the finished box: §663 abbreviates its list and §198 shows the
/// box itself.
pub(crate) fn report_pack_diagnostics(
    stores: &mut Universe,
    direction: PackedDirection,
    diagnostics: &[PackDiagnostic],
    packed: &Node,
) {
    for diagnostic in diagnostics {
        report_one(stores, direction, diagnostic, packed);
    }
}

fn report_one(
    stores: &mut Universe,
    direction: PackedDirection,
    diagnostic: &PackDiagnostic,
    packed: &Node,
) {
    let children = match packed {
        Node::HList(node) | Node::VList(node) => node.children,
        _ => unreachable!("hpack and vpack produce an hlist or a vlist"),
    };
    // §660 and §674 both open with `print_ln`, closing whatever partial line
    // the job was on, and then `print_nl` the headline.
    let mut headline = String::from("\n");
    match diagnostic {
        PackDiagnostic::Overfull { excess } => {
            headline.push_str("Overfull");
            headline.push_str(direction.box_name());
            headline.push_str(&tex_state::scaled::print_scaled(*excess));
            headline.push_str(direction.overfull_excess_text());
        }
        PackDiagnostic::Underfull { badness, .. } | PackDiagnostic::Loose { badness, .. } => {
            // §663 splits one stretching diagnostic into two names by how bad
            // it is: `if b>100 then print_nl("Underfull") else
            // print_nl("Loose")`.
            headline.push_str(if *badness > 100 { "Underfull" } else { "Loose" });
            headline.push_str(direction.box_name());
            headline.push_str("badness ");
            let _ = write!(headline, "{badness}");
        }
        PackDiagnostic::Tight { badness, .. } => {
            headline.push_str("Tight");
            headline.push_str(direction.box_name());
            headline.push_str("badness ");
            let _ = write!(headline, "{badness}");
        }
    }
    headline.push_str(&origin_text(stores));
    headline.push('\n');

    if direction == PackedDirection::Horizontal {
        // §663: `font_in_short_display:=null_font; short_display(list_ptr(r));
        // print_ln`. §675's vertical half has no such line.
        headline.push_str(&short_display(stores, children));
        headline.push('\n');
    }
    // TeX82 §§660/674 use the ordinary `print_ln`/`print_nl` primitives for
    // the headline and abbreviated hlist. Their live §54 selector is
    // therefore authoritative: batch mode writes the report to the log only,
    // while the other interaction modes write it to terminal and log.
    stores.printer().print_rendered(&headline);

    // §663/§675: `begin_diagnostic; show_box(r); end_diagnostic(true)`.
    // §198 shows the packed box itself, not its list: at `\showboxdepth`'s
    // default the children collapse to `[]`, which is exactly what makes the
    // dump one line rather than the whole subtree.
    let dump = dump_node_slice(
        stores,
        std::slice::from_ref(packed),
        DumpConfig::read(stores),
    );
    let mut scope = stores.begin_diagnostic();
    // §182 emits `print_ln` ahead of every node it shows, and §198 closes
    // with one of its own; `end_diagnostic(true)` supplies the last.
    scope.print_ln().print_rendered(&dump);
    scope.end(true);
}

/// §663's and §675's shared `<Finish issuing a diagnostic message...>`.
fn origin_text(stores: &Universe) -> String {
    if stores.output_routine_is_active() {
        return ") has occurred while \\output is active".to_owned();
    }
    let pack_begin_line = stores.pack_begin_line();
    let mut text = String::new();
    if pack_begin_line == 0 {
        text.push_str(") detected at line ");
    } else {
        // §661 records a paragraph's first line positively and an alignment's
        // negatively, which is the whole difference between the two phrasings.
        text.push_str(if pack_begin_line > 0 {
            ") in paragraph at lines "
        } else {
            ") in alignment at lines "
        });
        let _ = write!(text, "{}--", pack_begin_line.abs());
    }
    let _ = write!(text, "{}", stores.current_input_line());
    text
}

/// tex.web §174's `short_display`: the abbreviated form of an horizontal list.
///
/// Characters print as themselves, prefixed by their font's identifier
/// whenever the font changes; everything with internal structure collapses to
/// a placeholder. §175 fixes each placeholder: `[]` for a box-like node, `|`
/// for a rule, a space for non-zero glue, `$` for a math node, and a
/// discretionary's own pre-break and post-break text followed by skipping its
/// replacement count.
fn short_display(stores: &Universe, list: NodeListId) -> String {
    let mut out = String::new();
    let mut font = None;
    append_short_display(stores, list, &mut font, &mut out);
    out
}

pub(crate) fn short_display_nodes(stores: &Universe, nodes: &[Node]) -> String {
    let mut out = String::new();
    let mut font = None;
    append_short_display_nodes(stores, nodes, &mut font, &mut out);
    out
}

fn append_short_display(
    stores: &Universe,
    list: NodeListId,
    font_in_short_display: &mut Option<tex_state::ids::FontId>,
    out: &mut String,
) {
    let nodes = stores.nodes(list).to_vec();
    append_short_display_nodes(stores, &nodes, font_in_short_display, out);
}

fn append_short_display_nodes(
    stores: &Universe,
    nodes: &[Node],
    font_in_short_display: &mut Option<tex_state::ids::FontId>,
    out: &mut String,
) {
    for node in nodes {
        match node {
            Node::Char { font, ch, .. } => {
                append_short_char(stores, *font, *ch, font_in_short_display, out);
            }
            Node::Lig { orig, font, .. } => {
                // §175 recurses into `lig_ptr`, the original characters the
                // ligature replaced, not the ligature character itself.
                for original in orig {
                    append_short_char(stores, *font, *original, font_in_short_display, out);
                }
            }
            Node::HList(_)
            | Node::VList(_)
            | Node::Unset(_)
            | Node::Ins { .. }
            | Node::Whatsit(_)
            | Node::Mark { .. }
            | Node::Adjust(_) => out.push_str("[]"),
            Node::Rule { .. } => out.push('|'),
            Node::Glue { spec, .. } => {
                if stores.glue(*spec) != tex_state::glue::GlueSpec::ZERO {
                    out.push(' ');
                }
            }
            Node::MathOn(_) | Node::MathOff(_) => out.push('$'),
            Node::Disc { pre, post, .. } => {
                append_short_display(stores, *pre, font_in_short_display, out);
                append_short_display(stores, *post, font_in_short_display, out);
                // TeX82 steps past replacement nodes linked after a disc.
                // Umber stores them in the disc's side list, so iteration over
                // the containing list must not advance for their count.
            }
            // §175's `othercases do_nothing`: kerns, penalties, and the math
            // list nodes that never reach a packed horizontal list.
            _ => {}
        }
    }
}

fn append_short_char(
    stores: &Universe,
    font: tex_state::ids::FontId,
    ch: char,
    font_in_short_display: &mut Option<tex_state::ids::FontId>,
    out: &mut String,
) {
    if *font_in_short_display != Some(font) {
        out.push_str(&crate::node_dump::font_identifier(stores, font));
        out.push(' ');
        *font_in_short_display = Some(font);
    }
    out.push_str(&crate::node_dump::printable_char(ch));
}
