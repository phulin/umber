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
use tex_state::env::banks::IntParam;
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

/// Representation of discretionary replacement nodes in a diagnostic list.
///
/// Storage in the node arena does not determine this: paragraph diagnostics
/// freeze a detached projection before reporting, while ordinary packed lists
/// are frozen engine lists with TeX's physical replacement counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiagnosticListLayout {
    DetachedProjection,
    FrozenList,
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
pub(crate) fn report_pack_diagnostics<G>(
    stores: &mut Universe<G>,
    direction: PackedDirection,
    diagnostics: &[PackDiagnostic],
    packed: &Node,
    list_layout: DiagnosticListLayout,
) {
    for diagnostic in diagnostics {
        report_one(stores, direction, diagnostic, packed, list_layout);
    }
}

/// e-TeX [33.649]'s LR anomaly report followed by TeX82 §663's common
/// horizontal-box diagnostic tail.
pub(crate) fn report_lr_problems<G>(
    stores: &mut Universe<G>,
    missing: usize,
    extra: usize,
    packed: &Node,
    list_layout: DiagnosticListLayout,
) {
    let Node::HList(boxed) = packed else {
        unreachable!("LR recovery belongs to hpack")
    };
    let mut headline = format!("\n\\endL or \\endR problem ({missing} missing, {extra} extra");
    headline.push_str(&origin_text(stores));
    headline.push('\n');
    headline.push_str(&short_display(stores, boxed.children.clone(), list_layout));
    headline.push('\n');
    stores.printer().print_rendered(&headline);

    let dump = dump_node_slice(
        stores,
        std::slice::from_ref(packed),
        DumpConfig::read(stores),
    );
    let mut scope = stores.begin_diagnostic();
    scope.print_ln().print_rendered(&dump);
    scope.end(true);
}

fn report_one<G>(
    stores: &mut Universe<G>,
    direction: PackedDirection,
    diagnostic: &PackDiagnostic,
    packed: &Node,
    list_layout: DiagnosticListLayout,
) {
    let children = match packed {
        Node::HList(node) | Node::VList(node) => node.children.clone(),
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
    // TeX82 §675 puts its `print_ln` inside the non-output-active vbox
    // branch. During `\output`, §182's first `show_node_list` newline alone
    // terminates the headline. The hbox path always closes its headline in
    // §663 before printing the abbreviated list.
    if direction == PackedDirection::Horizontal || !stores.output_routine_is_active() {
        headline.push('\n');
    }

    if direction == PackedDirection::Horizontal {
        // §663: `font_in_short_display:=null_font; short_display(list_ptr(r));
        // print_ln`. §675's vertical half has no such line.
        headline.push_str(&short_display(stores, children, list_layout));
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
fn origin_text<G>(stores: &Universe<G>) -> String {
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
/// a placeholder. TeX82 §175 fixes each placeholder: `[]` for a box-like
/// node, `|` for a rule, a space for non-zero glue, `$` for a math node, and
/// a discretionary's own pre-break and post-break text followed by skipping
/// its replacement count. e-TeX 2.6's §175 change prints its L/R direction
/// subtypes as `[]` instead of ordinary math `$` markers.
fn short_display<G>(
    stores: &Universe<G>,
    list: tex_state::node_arena::PageListId,
    list_layout: DiagnosticListLayout,
) -> String {
    ShortDisplayRenderer::new().render_list_with_layout(stores, list, list_layout)
}

/// TeX82 §174's stateful `short_display` renderer.
///
/// `font_in_short_display` is deliberately retained across successive list
/// fragments until the owning caller resets it. Paragraph tracing uses one
/// renderer per §851 line-breaking pass; standalone packed-box diagnostics
/// create a fresh renderer for each box, matching their explicit reset.
pub(crate) struct ShortDisplayRenderer {
    // TeX82 §174 stores the internal font number. `FontId` additionally
    // carries an arena-owner namespace, which can differ across restored
    // format generations while its dense TeX font number remains the same.
    font: Option<u32>,
}

impl ShortDisplayRenderer {
    pub(crate) const fn new() -> Self {
        Self { font: None }
    }

    pub(crate) fn reset(&mut self) {
        self.font = None;
    }

    pub(crate) fn render_nodes<G>(&mut self, stores: &Universe<G>, nodes: &[Node]) -> String {
        let mut out = String::new();
        append_short_display_nodes(
            stores,
            nodes,
            DiscReplacementLayout::DetachedProjection,
            &mut self.font,
            &mut out,
        );
        out
    }

    pub(crate) fn render_line_break_trace_suffix<G>(
        &mut self,
        stores: &Universe<G>,
        list: tex_state::node_arena::PageListId,
    ) -> String {
        self.render_list_with_layout(stores, list, DiagnosticListLayout::FrozenList)
    }

    #[cfg(test)]
    fn render_list<G>(
        &mut self,
        stores: &Universe<G>,
        list: tex_state::node_arena::PageListId,
    ) -> String {
        self.render_list_with_layout(stores, list, DiagnosticListLayout::FrozenList)
    }

    fn render_list_with_layout<G>(
        &mut self,
        stores: &Universe<G>,
        list: tex_state::node_arena::PageListId,
        list_layout: DiagnosticListLayout,
    ) -> String {
        let mut out = String::new();
        append_short_display(stores, list, list_layout, &mut self.font, &mut out);
        out
    }
}

fn append_short_display<G>(
    stores: &Universe<G>,
    list: tex_state::node_arena::PageListId,
    list_layout: DiagnosticListLayout,
    font_in_short_display: &mut Option<u32>,
    out: &mut String,
) {
    let nodes = stores
        .page_node_list(list)
        .expect("diagnostic list belongs to the live page arena")
        .nodes()
        .to_vec();
    append_short_display_nodes(
        stores,
        &nodes,
        match list_layout {
            DiagnosticListLayout::DetachedProjection => DiscReplacementLayout::DetachedProjection,
            DiagnosticListLayout::FrozenList => DiscReplacementLayout::FrozenList,
        },
        font_in_short_display,
        out,
    );
}

#[derive(Clone, Copy)]
enum DiscReplacementLayout {
    /// Paragraph tracing projects TeX's mutable linked list into a detached
    /// slice. Its immutable side-list length identifies the projected nodes
    /// hidden after a discretionary.
    DetachedProjection,
    /// A frozen engine list carries TeX's actual `replace_count` explicitly.
    FrozenList,
}

fn append_short_display_nodes<G>(
    stores: &Universe<G>,
    nodes: &[Node],
    disc_layout: DiscReplacementLayout,
    font_in_short_display: &mut Option<u32>,
    out: &mut String,
) {
    let mut index = 0;
    while let Some(node) = nodes.get(index) {
        index += 1;
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
                if *spec != tex_state::glue::GlueSpec::ZERO {
                    out.push(' ');
                }
            }
            Node::MathOn(_)
            | Node::MathOff(_)
            | Node::Direction(
                tex_state::node::Direction::BeginM | tex_state::node::Direction::EndM,
            ) => out.push('$'),
            Node::Direction(_) => out.push_str("[]"),
            Node::Disc {
                pre,
                post,
                replace,
                physical_replace_count,
                ..
            } => {
                append_short_display(
                    stores,
                    pre.clone(),
                    DiagnosticListLayout::FrozenList,
                    font_in_short_display,
                    out,
                );
                append_short_display(
                    stores,
                    post.clone(),
                    DiagnosticListLayout::FrozenList,
                    font_in_short_display,
                    out,
                );
                // TeX82 §174 advances past the replacement nodes linked after
                // the discretionary. Frozen engine lists carry that count
                // explicitly; paragraph tracing instead supplies a detached
                // projection whose hidden suffix is described by the
                // immutable replacement side list.
                let replacement_count = match disc_layout {
                    DiscReplacementLayout::DetachedProjection => replace.len(),
                    DiscReplacementLayout::FrozenList => usize::from(*physical_replace_count),
                };
                index = index.saturating_add(replacement_count).min(nodes.len());
            }
            // §175's `othercases do_nothing`: kerns, penalties, and the math
            // list nodes that never reach a packed horizontal list.
            _ => {}
        }
    }
}

fn append_short_char<G>(
    stores: &Universe<G>,
    font: tex_state::ids::FontId,
    ch: char,
    font_in_short_display: &mut Option<u32>,
    out: &mut String,
) {
    if *font_in_short_display != Some(font.raw()) {
        out.push_str(&crate::node_dump::font_identifier(stores, font));
        out.push(' ');
        *font_in_short_display = Some(font.raw());
    }
    // TeX82 §§59/174 call `print(character(p))`, not `print_char` and not a
    // context-free `^^` renderer. A one-character string equal to the live
    // `\newlinechar` therefore performs `print_ln` before §59 considers the
    // non-printable-character spelling.
    let newline_char = u32::try_from(stores.int_param(IntParam::NEWLINE_CHAR))
        .ok()
        .and_then(char::from_u32);
    if newline_char == Some(ch) {
        out.push('\n');
    } else {
        out.push_str(&crate::node_dump::printable_char(ch));
    }
}
