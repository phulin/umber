use std::collections::BTreeMap;
use tex_expand::ExpansionHooks;
use tex_lex::{InputSource, InputStack};
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam};
use tex_state::glue::Order;
use tex_state::node::Node;
use tex_state::page::PageMark;
use tex_state::scaled::Scaled;
use tex_state::token::TracedTokenWord;
use tex_typeset::{PackSpec, VerticalBreakError, vert_break};

use crate::ExecError;
use crate::diagnostics;
use crate::packing_params::{vpack, vpack_params};
use crate::splitting::{prune_page_top_with_discards, vpack_natural};

use super::super::{scan_optional_keyword_x, scan_register_index, scan_scaled};

pub(super) fn scan_vsplit_node<S, H>(
    input: &mut InputStack<S>,
    stores: &mut Universe,
    hooks: &mut H,
    context: TracedTokenWord,
) -> Result<Option<Node>, ExecError>
where
    S: InputSource,
    H: ExpansionHooks<S>,
{
    stores.clear_split_discards();
    let index = scan_register_index(input, stores, hooks, context)?;
    if !scan_optional_keyword_x(input, stores, hooks, "to")? {
        // TeX.web §1082 inserts the keyword conceptually; keyword scanning
        // has already backed up the first nonmatching token, which is the
        // dimension's first token.
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! Missing `to' inserted.\nI'm working on `\\vsplit<box number> to <dimen>';\nwill look for the <dimen> next.\n",
        );
    }
    let height = scan_scaled(input, stores, hooks, context)?;
    split_vbox_register(stores, index, height)
}

fn split_vbox_register(
    stores: &mut Universe,
    index: u16,
    height: Scaled,
) -> Result<Option<Node>, ExecError> {
    let split_top_skip = stores.glue_param(GlueParam::SPLIT_TOP_SKIP);
    let split_max_depth = stores.dimen_param(DimenParam::SPLIT_MAX_DEPTH);
    let Some(source) = stores.box_reg(index) else {
        clear_split_marks(stores);
        return Ok(None);
    };
    let Some(source_node) = stores.nodes(source).first().map(|node| node.to_owned()) else {
        clear_split_marks(stores);
        stores.clear_box_reg_same_level(index);
        return Ok(None);
    };
    let Node::VList(source_box) = source_node else {
        clear_split_marks(stores);
        // TeX.web §977 leaves an hbox source untouched and returns a void
        // split result after the recoverable diagnostic.
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! \\vsplit needs a \\vbox.\nThe box you are trying to split is an \\hbox.\nI can't split such a box, so I'll leave it alone.\n",
        );
        return Ok(None);
    };

    let children = stores.clone_node_list_to_epoch(source_box.children);
    let mut split_nodes = stores.nodes(children).to_vec();
    let split =
        vert_break(stores, &split_nodes, height, split_max_depth).map_err(vertical_break_error)?;
    normalize_split_infinite_shrink(stores, &mut split_nodes, &split.infinite_shrink_glue);
    let remainder = match split.break_index {
        Some(index) => split_nodes.split_off(index),
        None => Vec::new(),
    };

    update_split_marks(stores, &split_nodes);
    replace_split_source(stores, index, remainder, split_top_skip);

    let split_list = stores.freeze_node_list(&split_nodes);
    let mut params = vpack_params(stores);
    params.box_max_depth = split_max_depth;
    Ok(Some(Node::VList(
        vpack(stores, split_list, PackSpec::Exactly(height), params).node,
    )))
}

fn normalize_split_infinite_shrink(stores: &mut Universe, nodes: &mut [Node], indices: &[usize]) {
    for &index in indices {
        let Some(Node::Glue { spec, kind, leader }) = nodes.get(index) else {
            continue;
        };
        let mut finite = stores.glue(*spec);
        if finite.shrink_order == Order::Normal || finite.shrink.raw() == 0 {
            continue;
        }
        diagnostics::report_split_infinite_shrinkage(stores);
        finite.shrink_order = Order::Normal;
        nodes[index] = Node::Glue {
            spec: stores.intern_glue(finite),
            kind: *kind,
            leader: *leader,
        };
    }
}

fn replace_split_source(
    stores: &mut Universe,
    index: u16,
    remainder: Vec<Node>,
    split_top_skip: tex_state::ids::GlueId,
) {
    let (pruned, discarded) = prune_page_top_with_discards(stores, remainder, split_top_skip);
    if stores.int_param(tex_state::env::banks::IntParam::SAVING_V_DISCARDS) > 0 {
        stores.set_split_discards(discarded);
    }
    if pruned.is_empty() {
        stores.clear_box_reg_same_level(index);
        return;
    }

    let remainder_list = stores.freeze_node_list(&pruned);
    let packed = vpack_natural(stores, remainder_list);
    let boxed = stores.freeze_node_list(&[Node::VList(packed)]);
    stores.set_box_reg_same_level(index, boxed);
}

fn update_split_marks(stores: &mut Universe, nodes: &[Node]) {
    clear_split_marks(stores);
    let mut classes = BTreeMap::new();
    for node in nodes {
        if let Node::Mark { class, tokens } = node {
            let (first, bot) = classes.entry(*class).or_insert((None, None));
            if first.is_none() {
                *first = Some(*tokens);
            }
            *bot = Some(*tokens);
        }
    }
    for (class, (first, bot)) in classes {
        stores.set_page_mark_class(
            PageMark::SplitFirst,
            class,
            first.unwrap_or(tex_state::ids::TokenListId::EMPTY),
        );
        stores.set_page_mark_class(
            PageMark::SplitBot,
            class,
            bot.unwrap_or(tex_state::ids::TokenListId::EMPTY),
        );
    }
}

fn clear_split_marks(stores: &mut Universe) {
    stores.set_page_mark(PageMark::SplitFirst, tex_state::ids::TokenListId::EMPTY);
    stores.set_page_mark(PageMark::SplitBot, tex_state::ids::TokenListId::EMPTY);
    let classes = stores.page_mark_classes().collect::<Vec<_>>();
    for class in classes {
        stores.set_page_mark_class(
            PageMark::SplitFirst,
            class,
            tex_state::ids::TokenListId::EMPTY,
        );
        stores.set_page_mark_class(
            PageMark::SplitBot,
            class,
            tex_state::ids::TokenListId::EMPTY,
        );
    }
}

fn vertical_break_error(error: VerticalBreakError) -> ExecError {
    match error {
        VerticalBreakError::ArithmeticOverflow => ExecError::ArithmeticOverflow,
    }
}
