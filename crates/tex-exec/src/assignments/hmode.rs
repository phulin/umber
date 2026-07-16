use tex_expand::get_x_token_with_context;
use tex_fonts::{LigKernChar, LigKernCommand};
use tex_lex::InputStack;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::FontId;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::node::{DiscKind, GlueKind, KernKind, Node};
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, Token};
use tex_state::{ExpansionState, PrintSink, Universe};
use tex_typeset::{INF_BAD, PackSpec, VpackParams};

use super::paragraph::{end_paragraph, ensure_horizontal_for_character, normal_paragraph};
use super::*;
use crate::dispatch::dispatch_delivered_token_with_context;
use crate::mode::PendingHRunChar;
use crate::packing_params::vpack;
use crate::vertical::{append_vertical_contribution, build_page_if_outer_vertical};
use crate::{DispatchAction, ExecError, Mode, ModeNest, push_traced_tokens};

pub(crate) fn try_append_character(
    nest: &mut ModeNest,
    traced: TracedTokenWord,
    stores: &mut Universe,
) -> Result<bool, ExecError> {
    let token = tex_expand::semantic_token(traced);
    match (nest.current_mode(), token) {
        (Mode::RestrictedHorizontal | Mode::Horizontal, Token::Char { ch, cat }) => {
            if cat == Catcode::Space {
                append_space(nest, stores)?;
            } else {
                append_hchar(nest, stores, ch, traced.origin());
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub(crate) fn append_given_char(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    ch: char,
    origin: OriginId,
) -> Result<(), ExecError> {
    match nest.current_mode() {
        Mode::RestrictedHorizontal | Mode::Horizontal => {
            append_hchar(nest, stores, ch, origin);
            Ok(())
        }
        Mode::Vertical | Mode::InternalVertical => {
            ensure_horizontal_for_character(nest, input, stores)?;
            append_hchar(nest, stores, ch, origin);
            Ok(())
        }
        mode => Err(ExecError::UnimplementedTypesetting {
            mode,
            token: Token::Char {
                ch,
                cat: Catcode::Other,
            },
            origin: OriginId::UNKNOWN,
            operation: "character",
        }),
    }
}

pub(crate) fn flush_pending_hchars(
    nest: &mut ModeNest,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    let insert_hyphen_discs = nest.current_mode() == Mode::Horizontal;
    flush_pending_hchar_run(nest, stores, insert_hyphen_discs);
    Ok(())
}

fn flush_pending_hchar_run(nest: &mut ModeNest, stores: &mut Universe, insert_hyphen_discs: bool) {
    let Some(pending) = nest.current_list().pending_hchars() else {
        return;
    };
    if is_ltr_shaping_font(stores, pending.first.font) && is_supported_script(pending.script) {
        let language = nest.current_list().hyphen_language();
        let breaks = if insert_hyphen_discs {
            super::hyphenation::candidate_positions_for_chars(
                stores,
                language,
                &pending.source,
                stores.int_param(IntParam::LEFT_HYPHEN_MIN).max(1) as usize,
                stores.int_param(IntParam::RIGHT_HYPHEN_MIN).max(1) as usize,
            )
        } else {
            Vec::new()
        };
        let shaped = shape_open_type_chars(stores, &pending.source, &breaks);
        let list = nest.current_list_mut();
        let removed = list.take_pending_hchars();
        debug_assert_eq!(removed, Some(pending));
        list.set_no_boundary(false);
        list.append(shaped);
        return;
    }
    let no_boundary = nest.current_list().no_boundary();
    let boundary = (!no_boundary)
        .then(|| boundary_command_node(stores, pending.first, true))
        .flatten()
        .map(|node| (pending.node_start, node));
    let right_boundary_kern = (!no_boundary)
        .then(|| right_boundary_kern(stores, &pending.current))
        .flatten();
    let disc = literal_hyphen_disc(stores, &pending.current, insert_hyphen_discs);
    let trailing_auto_kern = auto_kern(stores, &pending.current, None);
    let list = nest.current_list_mut();
    let removed = list.take_pending_hchars();
    debug_assert_eq!(removed, Some(pending.clone()));
    list.set_no_boundary(false);
    list.push_reconstituted(
        boundary,
        rechar_node(pending.current),
        disc,
        trailing_auto_kern,
    );
    if let Some(kern) = right_boundary_kern {
        list.push(kern);
    }
}

pub(super) fn execute_hmode_material(
    context: TracedTokenWord,
    primitive: UnexpandablePrimitive,
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    match primitive {
        UnexpandablePrimitive::Char => {
            let value = scan_i32(input, stores, execution, context)?;
            let ch = char::from_u32(value as u32).ok_or(ExecError::InvalidCode {
                context: "\\char",
                value,
            })?;
            append_given_char(nest, input, stores, ch, context.origin())?;
        }
        UnexpandablePrimitive::HFil
        | UnexpandablePrimitive::HFill
        | UnexpandablePrimitive::HSs
        | UnexpandablePrimitive::HFilNeg => {
            flush_pending_hchars(nest, stores)?;
            let spec = match primitive {
                UnexpandablePrimitive::HFil => infinite_glue(Order::Fil, false, false),
                UnexpandablePrimitive::HFill => infinite_glue(Order::Fill, false, false),
                UnexpandablePrimitive::HSs => infinite_glue(Order::Fil, false, true),
                UnexpandablePrimitive::HFilNeg => infinite_glue(Order::Fil, true, false),
                _ => unreachable!(),
            };
            let spec = stores.intern_glue(spec);
            nest.current_list_mut().push(Node::Glue {
                spec,
                kind: GlueKind::Normal,
                leader: None,
            });
        }
        UnexpandablePrimitive::Penalty => {
            flush_pending_hchars(nest, stores)?;
            let penalty = scan_i32(input, stores, execution, context)?;
            append_vertical_contribution(nest, stores, Node::Penalty(penalty));
            build_page_if_outer_vertical(nest, stores)?;
        }
        UnexpandablePrimitive::VRule => {
            flush_pending_hchars(nest, stores)?;
            if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
                ensure_horizontal_for_character(nest, input, stores)?;
            }
            nest.current_list_mut().push(scan_rule_node(
                input, stores, execution, primitive, context,
            )?);
            nest.current_list_mut().set_space_factor(1000);
        }
        UnexpandablePrimitive::ControlSpace => append_control_space(nest, input, stores)?,
        UnexpandablePrimitive::ItalicCorrection => append_italic_correction(nest, stores)?,
        UnexpandablePrimitive::Discretionary => {
            let math_mode = matches!(nest.current_mode(), Mode::Math | Mode::DisplayMath);
            flush_pending_hchars(nest, stores)?;
            let pre = scan_hlist_group(input, stores, execution, "\\discretionary pre")?;
            let post = scan_hlist_group(input, stores, execution, "\\discretionary post")?;
            let mut replace =
                scan_hlist_group(input, stores, execution, "\\discretionary replace")?;
            if math_mode && !stores.nodes(replace).is_empty() {
                stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    "\n! Illegal math \\discretionary.\nSorry: The third part of a discretionary break must be\nempty, in math formulas. I had to delete your third part.\n",
                );
                replace = stores.freeze_node_list(&[]);
            }
            nest.current_list_mut().push(Node::Disc {
                kind: DiscKind::Discretionary,
                pre,
                post,
                replace,
            });
        }
        UnexpandablePrimitive::DiscretionaryHyphen => {
            flush_pending_hchars(nest, stores)?;
            let font = stores.current_font();
            let hyphen = u8::try_from(stores.font_hyphen_char(font))
                .ok()
                .map(char::from)
                .unwrap_or('-');
            let pre = stores.freeze_node_list(&[Node::Char {
                font,
                ch: hyphen,
                origin: context.origin(),
            }]);
            let empty = stores.freeze_node_list(&[]);
            nest.current_list_mut().push(Node::Disc {
                kind: DiscKind::ExplicitHyphen,
                pre,
                post: empty,
                replace: empty,
            });
        }
        UnexpandablePrimitive::NoBoundary => nest.current_list_mut().set_no_boundary(true),
        UnexpandablePrimitive::SpaceFactor => {
            skip_optional_equals_x(input, stores, execution)?;
            let value = scan_i32(input, stores, execution, context)?;
            if !(1..=32767).contains(&value) {
                stores.world_mut().write_text(
                    tex_state::PrintSink::TerminalAndLog,
                    &format!(
                        "\n! Bad space factor ({value}).\nI allow only values in the range 1..32767 here.\n"
                    ),
                );
            } else {
                nest.current_list_mut().set_space_factor(value);
            }
        }
        UnexpandablePrimitive::Accent => {
            execute_accent(nest, input, stores, execution, context)?;
        }
        UnexpandablePrimitive::Mark | UnexpandablePrimitive::Marks => {
            flush_pending_hchars(nest, stores)?;
            let class = if primitive == UnexpandablePrimitive::Marks {
                let value = scan_i32(input, stores, execution, context)?;
                if (0..=32_767).contains(&value) {
                    value as u16
                } else {
                    stores.report_bad_register_code(value, 32_767);
                    0
                }
            } else {
                0
            };
            let tokens = scan_general_text_expanded_with_driver(
                input,
                &mut tex_state::ExpansionContext::new(stores),
                execution,
                context,
            )?;
            append_vertical_contribution(nest, stores, Node::Mark { class, tokens });
        }
        UnexpandablePrimitive::VAdjust => execute_vadjust(nest, input, stores, execution)?,
        UnexpandablePrimitive::Insert => execute_insert(nest, input, stores, execution, context)?,
        _ => unreachable!("caller restricts hmode material primitives"),
    }
    Ok(())
}

fn execute_insert(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores)?;
    let mut value = scan_i32(input, stores, execution, context)?;
    if !(0..=255).contains(&value) {
        return Err(ExecError::InvalidCode {
            context: "\\insert",
            value,
        });
    }
    if value == 255 {
        stores.world_mut().write_text(
            tex_state::PrintSink::TerminalAndLog,
            "\n! You can't \\insert255.\nI'm changing to \\insert0; box 255 is special.\n",
        );
        value = 0;
    }
    let opener =
        next_non_space_traced_x(input, stores, execution)?.ok_or(ExecError::MissingToken {
            context: "\\insert group",
        })?;
    if !has_catcode_meaning(
        stores,
        tex_expand::semantic_token(opener),
        Catcode::BeginGroup,
    ) {
        return Err(ExecError::MissingToken {
            context: "\\insert group",
        });
    }

    stores.enter_group_with_kind(tex_state::GroupKind::Insert);
    let box_group_depth = stores.execution_group_depth();
    let mut inner = ModeNest::new();
    inner.push(Mode::InternalVertical);
    normal_paragraph(&mut inner, stores);
    scan_box_group(&mut inner, input, stores, execution, box_group_depth)?;
    if inner.current_mode() == Mode::Horizontal {
        end_paragraph(&mut inner, stores)?;
    }
    let level = inner.pop()?;
    let content = stores.freeze_node_list(level.list().nodes());
    let packed = vpack(
        stores,
        content,
        PackSpec::Natural,
        VpackParams {
            vbadness: INF_BAD,
            vfuzz: Scaled::MAX_DIMEN,
            box_max_depth: Scaled::MAX_DIMEN,
        },
    );
    let size = packed
        .node
        .height
        .checked_add(packed.node.depth)
        .ok_or(ExecError::ArithmeticOverflow)?;
    let split_top_skip = stores.glue_param(GlueParam::SPLIT_TOP_SKIP);
    let split_max_depth = stores.dimen_param(DimenParam::SPLIT_MAX_DEPTH);
    let floating_penalty = stores.int_param(IntParam::FLOATING_PENALTY);

    crate::leave_group(input, stores, tex_state::GroupKind::Insert)?;

    append_vertical_contribution(
        nest,
        stores,
        Node::Ins {
            class: value as u16,
            size,
            split_top_skip,
            split_max_depth,
            floating_penalty,
            content,
        },
    );
    build_page_if_outer_vertical(nest, stores)?;
    Ok(())
}

fn execute_vadjust(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
) -> Result<(), ExecError> {
    if !matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal | Mode::Math | Mode::DisplayMath
    ) {
        return Err(ExecError::UnimplementedTypesetting {
            mode: nest.current_mode(),
            token: Token::Cs(stores.intern("vadjust").symbol()),
            origin: OriginId::UNKNOWN,
            operation: "\\vadjust",
        });
    }
    if matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ) {
        flush_pending_hchars(nest, stores)?;
    }
    let opener = next_non_space_x(input, stores, execution)?.ok_or(ExecError::MissingToken {
        context: "\\vadjust group",
    })?;
    if !is_begin_group(opener) {
        return Err(ExecError::MissingToken {
            context: "\\vadjust group",
        });
    }
    stores.enter_group_with_kind(tex_state::GroupKind::AdjustedHBox);
    let box_group_depth = stores.execution_group_depth();
    let mut inner = ModeNest::new();
    inner.push(Mode::InternalVertical);
    normal_paragraph(&mut inner, stores);
    scan_box_group(&mut inner, input, stores, execution, box_group_depth)?;
    if inner.current_mode() == Mode::Horizontal {
        end_paragraph(&mut inner, stores)?;
    }
    let level = inner.pop()?;
    let content = stores.freeze_node_list(level.list().nodes());
    crate::leave_group(input, stores, tex_state::GroupKind::AdjustedHBox)?;
    nest.current_list_mut().push(Node::Adjust(content));
    Ok(())
}

fn append_space(nest: &mut ModeNest, stores: &mut Universe) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores)?;
    let configuration = stores.pdf_font_configuration();
    let sf = if configuration.adjusts_interword_glue() {
        1000
    } else {
        nest.current_list().space_factor()
    };
    let mut spec = if sf >= 2000 {
        nonzero_glue_param_or_font_space(stores, GlueParam::XSPACE_SKIP, sf)
    } else {
        nonzero_glue_param_or_font_space(stores, GlueParam::SPACE_SKIP, sf)
    };
    if configuration.adjusts_interword_glue() {
        adjust_interword_glue(stores, nest.current_list().nodes(), &mut spec);
    }
    let id = stores.intern_glue(spec);
    nest.current_list_mut().push(Node::Glue {
        spec: id,
        kind: GlueKind::Normal,
        leader: None,
    });
    Ok(())
}

fn append_control_space(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
) -> Result<(), ExecError> {
    if matches!(nest.current_mode(), Mode::Vertical | Mode::InternalVertical) {
        ensure_horizontal_for_character(nest, input, stores)?;
    }
    flush_pending_hchars(nest, stores)?;
    let mut spec = nonzero_glue_param_or_font_space(stores, GlueParam::SPACE_SKIP, 1000);
    if stores.pdf_font_configuration().adjusts_interword_glue() {
        adjust_interword_glue(stores, nest.current_list().nodes(), &mut spec);
    }
    let id = stores.intern_glue(spec);
    nest.current_list_mut().push(Node::Glue {
        spec: id,
        kind: GlueKind::Normal,
        leader: None,
    });
    Ok(())
}

fn append_hchar(nest: &mut ModeNest, stores: &mut Universe, ch: char, origin: OriginId) {
    if nest.current_mode() == Mode::Horizontal {
        let language = u8::try_from(stores.int_param(IntParam::LANGUAGE)).unwrap_or(0);
        if language != nest.current_list().hyphen_language() {
            // tex.web's fix_language flushes the current ligature word before
            // recording the new language and its current hyphen minima.
            flush_pending_hchar_run(nest, stores, true);
            let left_hyphen_min = stores.int_param(IntParam::LEFT_HYPHEN_MIN).clamp(1, 63) as u8;
            let right_hyphen_min = stores.int_param(IntParam::RIGHT_HYPHEN_MIN).clamp(1, 63) as u8;
            nest.current_list_mut()
                .push(Node::Whatsit(tex_state::node::Whatsit::Language {
                    language,
                    left_hyphen_min,
                    right_hyphen_min,
                }));
            nest.current_list_mut().set_hyphen_language(language);
        }
    }
    let font = stores.current_font();
    if stores.font_character_exists(font, ch) {
        if let Some(pending) = nest.current_list().pending_hchars()
            && (is_ltr_shaping_font(stores, font)
                || is_ltr_shaping_font(stores, pending.first.font))
            && (pending.first.font != font
                || !scripts_compatible(pending.script, tex_shape::character_script(ch)))
        {
            let insert_hyphen_discs = nest.current_mode() == Mode::Horizontal;
            flush_pending_hchar_run(nest, stores, insert_hyphen_discs);
        }
        append_pending_hchar(nest, stores, font, ch, origin);
        update_space_factor(nest, stores, ch);
        return;
    }
    report_missing_character(stores, font, ch);
}

fn append_pending_hchar(
    nest: &mut ModeNest,
    stores: &mut Universe,
    font: FontId,
    ch: char,
    origin: OriginId,
) {
    let Some(mut pending) = nest.current_list().pending_hchars() else {
        if let Some(kern) = auto_kern(stores, &PendingHRunChar::new(font, ch, origin), Some(true)) {
            nest.current_list_mut().push(kern);
        }
        nest.current_list_mut()
            .begin_pending_hchars(font, ch, origin);
        return;
    };
    if is_ltr_shaping_font(stores, font)
        && is_supported_script(pending.script)
        && is_supported_script(tex_shape::character_script(ch))
    {
        let script = tex_shape::character_script(ch);
        if is_strong_script(script) {
            pending.script = script;
        }
        pending
            .source
            .push(crate::mode::PendingHChar { font, ch, origin });
        pending.current = PendingHRunChar::new(font, ch, origin);
        nest.current_list_mut().set_pending_hchars(pending);
        return;
    }
    let next = PendingHRunChar::new(font, ch, origin);
    let emitted = match reconstitution_step(stores, pending.current, next.clone()) {
        ReconstitutionStep::Merge(merged) => {
            pending.current = merged;
            None
        }
        ReconstitutionStep::Emit { current, kern } => {
            let insert_hyphen_discs = nest.current_mode() == Mode::Horizontal;
            let disc = literal_hyphen_disc(stores, &current, insert_hyphen_discs);
            let auto = auto_kern_between(stores, &current, &next);
            let font_kern = kern.map(|amount| Node::Kern {
                amount,
                kind: KernKind::Font,
            });
            pending.current = next;
            Some((rechar_node(current), disc, auto, font_kern))
        }
    };
    let list = nest.current_list_mut();
    if let Some((current, disc, auto, font_kern)) = emitted {
        list.push(current);
        if let Some(disc) = disc {
            list.push(disc);
        }
        if let Some(auto) = auto {
            list.push(auto);
        }
        if let Some(font_kern) = font_kern {
            list.push(font_kern);
        }
    }
    list.set_pending_hchars(pending);
}

fn is_strong_script(script: tex_shape::Script) -> bool {
    !matches!(
        script,
        tex_shape::Script::Common | tex_shape::Script::Inherited | tex_shape::Script::Unknown
    )
}

fn scripts_compatible(left: tex_shape::Script, right: tex_shape::Script) -> bool {
    !is_strong_script(left) || !is_strong_script(right) || left == right
}

fn is_supported_script(script: tex_shape::Script) -> bool {
    matches!(
        script,
        tex_shape::Script::Common
            | tex_shape::Script::Inherited
            | tex_shape::Script::Latin
            | tex_shape::Script::Cyrillic
            | tex_shape::Script::Greek
            | tex_shape::Script::Han
            | tex_shape::Script::Hiragana
            | tex_shape::Script::Katakana
            | tex_shape::Script::Hangul
            | tex_shape::Script::Bopomofo
    )
}

fn is_ltr_shaping_font(stores: &Universe, font: FontId) -> bool {
    stores.font(font).shaping_font().is_some()
        && stores.font(font).shaping_direction() == Some(tex_fonts::WritingDirection::LeftToRight)
}

fn shape_open_type_chars(
    stores: &Universe,
    chars: &[crate::mode::PendingHChar],
    break_positions: &[usize],
) -> Vec<Node> {
    use std::collections::BTreeMap;

    let Some(first) = chars.first() else {
        return Vec::new();
    };
    let font = stores.font(first.font);
    let shaping_font = font.shaping_font().expect("OpenType run font");
    let features = font.shaping_features().expect("OpenType feature policy");
    let text = chars.iter().map(|entry| entry.ch).collect::<String>();
    let byte_starts = text
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let break_bytes = break_positions
        .iter()
        .filter_map(|&position| byte_starts.get(position).copied())
        .collect::<Vec<_>>();
    let shaped = tex_shape::shape_run_with_breaks(
        shaping_font,
        &text,
        features,
        tex_shape::Direction::LeftToRight,
        &break_bytes,
    );
    let mut cluster_advances = BTreeMap::<usize, i64>::new();
    for glyph in shaped.glyphs {
        *cluster_advances.entry(glyph.cluster as usize).or_default() +=
            i64::from(glyph.x_advance.raw());
    }
    let cluster_starts = cluster_advances.keys().copied().collect::<Vec<_>>();
    let mut adjustments = vec![Scaled::from_raw(0); chars.len()];
    for (cluster_index, &start_byte) in cluster_starts.iter().enumerate() {
        let end_byte = cluster_starts
            .get(cluster_index + 1)
            .copied()
            .unwrap_or(text.len());
        let start = byte_starts.partition_point(|&byte| byte < start_byte);
        let end = byte_starts.partition_point(|&byte| byte < end_byte);
        if start >= end {
            continue;
        }
        let nominal = chars[start..end].iter().fold(0_i64, |sum, entry| {
            sum + i64::from(
                stores
                    .font_character_metrics(entry.font, entry.ch)
                    .map_or(0, |metrics| metrics.width.raw()),
            )
        });
        let shaped = cluster_advances[&start_byte];
        adjustments[end - 1] = Scaled::from_raw(
            i32::try_from(shaped - nominal).expect("shaped cluster adjustment fits Scaled"),
        );
    }
    let mut nodes = Vec::with_capacity(chars.len() * 2);
    for (entry, adjustment) in chars.iter().zip(adjustments) {
        nodes.push(Node::Char {
            font: entry.font,
            ch: entry.ch,
            origin: entry.origin,
        });
        if adjustment.raw() != 0 {
            nodes.push(Node::Kern {
                amount: adjustment,
                kind: KernKind::Font,
            });
        }
    }
    nodes
}

/// Replaces provisional OpenType shaping adjustments in a materialized list.
///
/// Every call shapes caller-delimited runs independently. Paragraph code uses
/// this after break selection, which restores ligatures on each unsplit side
/// while preventing a glyph cluster from crossing the chosen line boundary.
pub(super) fn reshape_open_type_runs(stores: &Universe, nodes: &mut Vec<Node>) {
    let mut index = 0;
    while index < nodes.len() {
        let Node::Char { font, ch, origin } = nodes[index] else {
            index += 1;
            continue;
        };
        if !is_ltr_shaping_font(stores, font)
            || !is_supported_script(tex_shape::character_script(ch))
        {
            index += 1;
            continue;
        }
        let mut chars = vec![crate::mode::PendingHChar { font, ch, origin }];
        let mut script = tex_shape::character_script(ch);
        let start = index;
        index += 1;
        while index < nodes.len() {
            match nodes[index] {
                Node::Kern {
                    kind: KernKind::Font,
                    ..
                } => index += 1,
                Node::Char {
                    font: next_font,
                    ch: next_ch,
                    origin: next_origin,
                } if next_font == font
                    && scripts_compatible(script, tex_shape::character_script(next_ch)) =>
                {
                    let next_script = tex_shape::character_script(next_ch);
                    if is_strong_script(next_script) {
                        script = next_script;
                    }
                    chars.push(crate::mode::PendingHChar {
                        font,
                        ch: next_ch,
                        origin: next_origin,
                    });
                    index += 1;
                }
                _ => break,
            }
        }
        let shaped = shape_open_type_chars(stores, &chars, &[]);
        let shaped_len = shaped.len();
        nodes.splice(start..index, shaped);
        index = start + shaped_len;
    }
}

pub(crate) fn reconstitute(
    stores: &mut Universe,
    pending: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
    insert_hyphen_discs: bool,
) -> Vec<Node> {
    let mut entries = pending.iter().copied();
    let Some(first) = entries.next() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(pending.len());
    if let Some(kern) = auto_kern(
        stores,
        &PendingHRunChar::new(first.font, first.ch, first.origin),
        Some(true),
    ) {
        out.push(kern);
    }
    if !no_left_boundary && let Some(node) = boundary_command_node(stores, first, true) {
        out.push(node);
    }
    let mut current = PendingHRunChar::new(first.font, first.ch, first.origin);
    for entry in entries {
        let next = PendingHRunChar::new(entry.font, entry.ch, entry.origin);
        match reconstitution_step(stores, current, next.clone()) {
            ReconstitutionStep::Merge(merged) => current = merged,
            ReconstitutionStep::Emit {
                current: emitted,
                kern,
            } => {
                let auto = auto_kern_between(stores, &emitted, &next);
                if let Some(disc) = literal_hyphen_disc(stores, &emitted, insert_hyphen_discs) {
                    out.push(rechar_node(emitted));
                    out.push(disc);
                } else {
                    out.push(rechar_node(emitted));
                }
                if let Some(amount) = kern {
                    if let Some(auto) = auto {
                        out.push(auto);
                    }
                    out.push(Node::Kern {
                        amount,
                        kind: KernKind::Font,
                    });
                } else if let Some(auto) = auto {
                    out.push(auto);
                }
                current = next;
            }
        }
    }
    let disc = literal_hyphen_disc(stores, &current, insert_hyphen_discs);
    let trailing_auto_kern = auto_kern(stores, &current, None);
    out.push(rechar_node(current));
    if let Some(disc) = disc {
        out.push(disc);
    }
    if let Some(kern) = trailing_auto_kern {
        out.push(kern);
    }
    out
}

fn auto_kern_between(
    stores: &Universe,
    left: &PendingHRunChar,
    right: &PendingHRunChar,
) -> Option<Node> {
    if left.font == right.font {
        return auto_kern_codes(stores, left.font, Some(left.ch), Some(right.ch));
    }
    // Font changes normally flush the old run before the assignment. Keep the
    // fallback deterministic for reconstructed mixed-font runs by applying
    // only the old font's trailing append code here.
    auto_kern_codes(stores, left.font, Some(left.ch), None)
}

fn auto_kern(stores: &Universe, glyph: &PendingHRunChar, leading: Option<bool>) -> Option<Node> {
    match leading {
        Some(true) => auto_kern_codes(stores, glyph.font, None, Some(glyph.ch)),
        _ => auto_kern_codes(stores, glyph.font, Some(glyph.ch), None),
    }
}

fn auto_kern_codes(
    stores: &Universe,
    font: FontId,
    left: Option<char>,
    right: Option<char>,
) -> Option<Node> {
    let configuration = stores.pdf_font_configuration();
    let mut amount = Scaled::from_raw(0);
    if configuration.appends_kerns()
        && let Some(left) = left.and_then(|ch| u8::try_from(ch as u32).ok())
    {
        amount = add_scaled(
            amount,
            scaled_font_code(
                stores,
                font,
                stores.pdf_font_code(tex_state::PdfFontCode::Knac, font, left),
            ),
        );
    }
    if configuration.prepends_kerns()
        && let Some(right) = right.and_then(|ch| u8::try_from(ch as u32).ok())
    {
        amount = add_scaled(
            amount,
            scaled_font_code(
                stores,
                font,
                stores.pdf_font_code(tex_state::PdfFontCode::Knbc, font, right),
            ),
        );
    }
    (amount.raw() != 0).then_some(Node::Kern {
        amount,
        kind: KernKind::Auto,
    })
}

fn add_scaled(left: Scaled, right: Scaled) -> Scaled {
    left.checked_add(right)
        .expect("pdfTeX inter-character kern adjustment fits Scaled")
}

fn adjust_interword_glue(stores: &Universe, nodes: &[Node], spec: &mut GlueSpec) {
    let mut glyph = None;
    for node in nodes.iter().rev() {
        match node {
            Node::Char { font, ch, .. } | Node::Lig { font, ch, .. } => {
                glyph = u8::try_from(*ch as u32).ok().map(|code| (*font, code));
                break;
            }
            Node::Kern {
                kind: KernKind::Auto,
                ..
            } => {}
            _ => return,
        }
    }
    let Some((font, code)) = glyph else {
        return;
    };
    let width = scaled_font_code(
        stores,
        font,
        stores.pdf_font_code(tex_state::PdfFontCode::Knbs, font, code),
    );
    let stretch = scaled_font_code(
        stores,
        font,
        stores.pdf_font_code(tex_state::PdfFontCode::Stbs, font, code),
    );
    let shrink = scaled_font_code(
        stores,
        font,
        stores.pdf_font_code(tex_state::PdfFontCode::Shbs, font, code),
    );
    spec.width = spec
        .width
        .checked_add(width)
        .expect("pdfTeX interword width adjustment fits Scaled");
    spec.stretch = spec
        .stretch
        .checked_add(stretch)
        .expect("pdfTeX interword stretch adjustment fits Scaled");
    spec.shrink = spec
        .shrink
        .checked_add(shrink)
        .expect("pdfTeX interword shrink adjustment fits Scaled");
}

fn scaled_font_code(stores: &Universe, font: FontId, code: i32) -> Scaled {
    let product = i64::from(stores.font_parameter(font, 6).raw()) * i64::from(code);
    let rounded = if product >= 0 {
        (product + 500) / 1000
    } else {
        -((-product + 500) / 1000)
    };
    Scaled::from_raw(i32::try_from(rounded).unwrap_or(if rounded < 0 {
        i32::MIN
    } else {
        i32::MAX
    }))
}

enum ReconstitutionStep {
    Merge(PendingHRunChar),
    Emit {
        current: PendingHRunChar,
        kern: Option<Scaled>,
    },
}

fn reconstitution_step(
    stores: &Universe,
    current: PendingHRunChar,
    next: PendingHRunChar,
) -> ReconstitutionStep {
    if current.font == next.font
        && stores.font_uses_tfm_metrics(current.font)
        && let (Ok(left), Ok(right)) = (font_code(current.ch), font_code(next.ch))
        && let Some(command) = stores.lig_kern_command(
            current.font,
            LigKernChar::Char(left),
            LigKernChar::Char(right),
        )
    {
        return match command {
            LigKernCommand::Kern(amount) => ReconstitutionStep::Emit {
                current,
                kern: Some(amount),
            },
            LigKernCommand::Ligature(lig) if lig.delete_next => {
                let mut orig = current.orig;
                orig.extend(next.orig);
                let mut origins = current.origins;
                origins.extend(next.origins);
                ReconstitutionStep::Merge(PendingHRunChar {
                    font: current.font,
                    ch: char::from(lig.replacement),
                    orig,
                    origins,
                    ligature_present: true,
                })
            }
            LigKernCommand::Ligature(_) => ReconstitutionStep::Emit {
                current,
                kern: None,
            },
        };
    }
    ReconstitutionStep::Emit {
        current,
        kern: None,
    }
}

fn rechar_node(current: PendingHRunChar) -> Node {
    if current.ligature_present {
        Node::Lig {
            font: current.font,
            ch: current.ch,
            orig: current.orig,
            origins: current.origins,
        }
    } else {
        Node::Char {
            font: current.font,
            ch: current.ch,
            origin: current
                .origins
                .first()
                .copied()
                .unwrap_or(OriginId::UNKNOWN),
        }
    }
}

fn literal_hyphen_disc(
    stores: &mut Universe,
    current: &PendingHRunChar,
    enabled: bool,
) -> Option<Node> {
    if !enabled
        || stores.font_hyphen_char(current.font)
            != current.orig.last().copied().unwrap_or(current.ch) as i32
    {
        return None;
    }
    let empty = stores.freeze_node_list(&[]);
    Some(Node::Disc {
        kind: DiscKind::ExplicitHyphen,
        pre: empty,
        post: empty,
        replace: empty,
    })
}

fn boundary_command_node(
    stores: &Universe,
    current: crate::mode::PendingHChar,
    left: bool,
) -> Option<Node> {
    if !stores.font_uses_tfm_metrics(current.font) {
        return None;
    }
    let code = font_code(current.ch).ok()?;
    let command = if left {
        stores.lig_kern_command(current.font, LigKernChar::Boundary, LigKernChar::Char(code))?
    } else {
        stores.lig_kern_command(current.font, LigKernChar::Char(code), LigKernChar::Boundary)?
    };
    match command {
        LigKernCommand::Kern(amount) => Some(Node::Kern {
            amount,
            kind: KernKind::Font,
        }),
        LigKernCommand::Ligature(lig) => Some(Node::Lig {
            font: current.font,
            ch: char::from(lig.replacement),
            orig: vec![current.ch],
            origins: vec![current.origin],
        }),
    }
}

fn right_boundary_kern(stores: &Universe, current: &PendingHRunChar) -> Option<Node> {
    if !stores.font_uses_tfm_metrics(current.font) {
        return None;
    }
    let code = font_code(current.ch).ok()?;
    match stores.lig_kern_command(current.font, LigKernChar::Char(code), LigKernChar::Boundary)? {
        LigKernCommand::Kern(amount) => Some(Node::Kern {
            amount,
            kind: KernKind::Font,
        }),
        LigKernCommand::Ligature(_) => None,
    }
}

fn update_space_factor(nest: &mut ModeNest, stores: &Universe, ch: char) {
    let sf = i32::from(stores.sfcode(ch));
    if sf == 0 {
        return;
    }
    let current = nest.current_list().space_factor();
    let next = if sf > 1000 && current < 1000 {
        1000
    } else {
        sf
    };
    nest.current_list_mut().set_space_factor(next);
}

fn nonzero_glue_param_or_font_space(
    stores: &Universe,
    override_param: GlueParam,
    space_factor: i32,
) -> GlueSpec {
    let override_spec = stores.glue(stores.glue_param(override_param));
    if override_spec != GlueSpec::ZERO {
        return override_spec;
    }
    let font = stores.current_font();
    let mut spec = GlueSpec {
        width: stores.font_parameter(font, 2),
        stretch: stores.font_parameter(font, 3),
        stretch_order: Order::Normal,
        shrink: stores.font_parameter(font, 4),
        shrink_order: Order::Normal,
    };
    if space_factor >= 2000 {
        spec.width = spec
            .width
            .checked_add(stores.font_parameter(font, 7))
            .unwrap_or(spec.width);
    }
    if space_factor != 1000 {
        spec.stretch = scale_by_factor(spec.stretch, space_factor, 1000);
        spec.shrink = scale_by_factor(spec.shrink, 1000, space_factor);
    }
    spec
}

fn scale_by_factor(value: Scaled, num: i32, den: i32) -> Scaled {
    Scaled::from_raw(((i64::from(value.raw()) * i64::from(num)) / i64::from(den)) as i32)
}

pub(super) fn infinite_glue(order: Order, negative: bool, shrink: bool) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(0),
        stretch: Scaled::from_raw(if negative {
            -Scaled::UNITY
        } else {
            Scaled::UNITY
        }),
        stretch_order: order,
        shrink: if shrink {
            Scaled::from_raw(Scaled::UNITY)
        } else {
            Scaled::from_raw(0)
        },
        shrink_order: if shrink { order } else { Order::Normal },
    }
}

pub(crate) fn fixed_infinite_glue(primitive: UnexpandablePrimitive) -> GlueSpec {
    match primitive {
        UnexpandablePrimitive::HFil | UnexpandablePrimitive::VFil => {
            infinite_glue(Order::Fil, false, false)
        }
        UnexpandablePrimitive::HFill | UnexpandablePrimitive::VFill => {
            infinite_glue(Order::Fill, false, false)
        }
        UnexpandablePrimitive::HSs | UnexpandablePrimitive::VSs => {
            infinite_glue(Order::Fil, false, true)
        }
        UnexpandablePrimitive::HFilNeg | UnexpandablePrimitive::VFilNeg => {
            infinite_glue(Order::Fil, true, false)
        }
        _ => unreachable!("caller restricts fixed infinite glue primitives"),
    }
}

fn report_missing_character(stores: &mut Universe, font: tex_state::ids::FontId, ch: char) {
    if stores.int_param(IntParam::new(36)) <= 0 {
        return;
    }
    let text = format!(
        "Missing character: There is no {} in font {}!\n",
        ch.escape_default(),
        stores.font_name(font)
    );
    stores
        .world_mut()
        .write_text(PrintSink::TerminalAndLog, &text);
}

fn execute_accent(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores)?;
    let accent_value = scan_i32(input, stores, execution, context)?;
    let accent = u8::try_from(accent_value).map_err(|_| ExecError::InvalidCode {
        context: "\\accent",
        value: accent_value,
    })?;
    let accent_font = stores.current_font();
    let Some(accent_metrics) = stores.font_char_metrics(accent_font, accent) else {
        report_missing_character(stores, accent_font, char::from(accent));
        return Ok(());
    };
    let base = scan_accent_base(nest, input, stores, execution, context)?;
    let Some(base) = base else {
        nest.current_list_mut().push(Node::Char {
            font: accent_font,
            ch: char::from(accent),
            origin: context.origin(),
        });
        return Ok(());
    };
    let base_font = stores.current_font();
    let Some(base_metrics) = stores.font_char_metrics(base_font, base) else {
        report_missing_character(stores, base_font, char::from(base));
        nest.current_list_mut().push(Node::Char {
            font: accent_font,
            ch: char::from(accent),
            origin: context.origin(),
        });
        nest.current_list_mut().set_space_factor(1000);
        return Ok(());
    };
    let accent_x_height = stores.font_parameter(accent_font, 5);
    let accent_slant = stores.font_parameter(accent_font, 1);
    let base_slant = stores.font_parameter(base_font, 1);
    let delta = tex_state::scaled::text_accent_delta(
        base_metrics.width,
        accent_metrics.width,
        base_metrics.height,
        base_slant,
        accent_x_height,
        accent_slant,
    );
    nest.current_list_mut().push(Node::Kern {
        amount: delta,
        kind: KernKind::Accent,
    });
    let accent_node = Node::Char {
        font: accent_font,
        ch: char::from(accent),
        origin: context.origin(),
    };
    if base_metrics.height == accent_x_height {
        nest.current_list_mut().push(accent_node);
    } else {
        let children = stores.freeze_node_list(&[accent_node]);
        let mut boxed = super::boxes::hpack_with_overfull_rule(stores, children, PackSpec::Natural);
        boxed.shift = accent_x_height
            .checked_sub(base_metrics.height)
            .ok_or(ExecError::ArithmeticOverflow)?;
        nest.current_list_mut().push(Node::HList(boxed));
    }
    let back = Scaled::from_raw(-accent_metrics.width.raw() - delta.raw());
    nest.current_list_mut().push(Node::Kern {
        amount: back,
        kind: KernKind::Accent,
    });
    nest.current_list_mut().push(Node::Char {
        font: base_font,
        ch: char::from(base),
        origin: context.origin(),
    });
    nest.current_list_mut().set_space_factor(1000);
    Ok(())
}

fn scan_accent_base(
    nest: &mut ModeNest,
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: TracedTokenWord,
) -> Result<Option<u8>, ExecError> {
    loop {
        let Some(traced) = get_x_token_with_context(
            input,
            &mut tex_state::ExpansionContext::new(stores),
            execution,
        )?
        else {
            return Ok(None);
        };
        let token = tex_expand::semantic_token(traced);
        if is_space(token) {
            continue;
        }
        let meaning = match token {
            Token::Cs(symbol) => Some(stores.meaning(symbol)),
            Token::Char {
                ch,
                cat: Catcode::Active,
            } => {
                let symbol = active_character_symbol(stores, ch);
                Some(stores.meaning(symbol))
            }
            Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => None,
        };
        if meaning == Some(Meaning::Relax) {
            continue;
        }
        if meaning.is_some_and(is_accent_assignment_meaning) {
            match dispatch_delivered_token_with_context(nest, traced, input, stores, execution)? {
                DispatchAction::Continue => continue,
                DispatchAction::End | DispatchAction::Shipout(_) | DispatchAction::NotConsumed => {
                    unreachable!("TeX82 do_assignments only dispatches ordinary assignments")
                }
            }
        }
        let ch = match (token, meaning) {
            (
                Token::Char {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                },
                _,
            )
            | (_, Some(Meaning::CharGiven(ch)))
            | (
                _,
                Some(Meaning::CharToken {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                }),
            ) => ch,
            (_, Some(Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char))) => {
                let value = scan_i32(input, stores, execution, context)?;
                let ch = u8::try_from(value).map_err(|_| ExecError::InvalidCode {
                    context: "\\accent base",
                    value,
                })?;
                return Ok(Some(ch));
            }
            _ => {
                push_traced_tokens(input, stores, [traced]);
                return Ok(None);
            }
        };
        return u8::try_from(ch as u32)
            .map(Some)
            .map_err(|_| ExecError::InvalidCode {
                context: "\\accent base",
                value: ch as i32,
            });
    }
}

fn is_accent_assignment_meaning(meaning: Meaning) -> bool {
    if matches!(meaning, Meaning::Font(_)) {
        return true;
    }
    if !is_assignment_meaning(meaning) {
        return false;
    }
    !matches!(
        meaning,
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::BeginGroup
                | UnexpandablePrimitive::EndGroup
                | UnexpandablePrimitive::AfterGroup
                | UnexpandablePrimitive::AfterAssignment
                | UnexpandablePrimitive::OpenIn
                | UnexpandablePrimitive::CloseIn
                | UnexpandablePrimitive::OpenOut
                | UnexpandablePrimitive::CloseOut
                | UnexpandablePrimitive::Immediate
                | UnexpandablePrimitive::Write
        )
    )
}

pub(crate) fn scan_rule_node(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    primitive: UnexpandablePrimitive,
    context: TracedTokenWord,
) -> Result<Node, ExecError> {
    let default_rule = Scaled::from_raw(26_214);
    let (mut width, mut height, mut depth) = if primitive == UnexpandablePrimitive::VRule {
        (Some(default_rule), None, None)
    } else {
        (None, Some(default_rule), Some(Scaled::from_raw(0)))
    };
    loop {
        if scan_optional_keyword_x(input, stores, execution, "width")? {
            width = Some(scan_scaled(input, stores, execution, context)?);
        } else if scan_optional_keyword_x(input, stores, execution, "height")? {
            height = Some(scan_scaled(input, stores, execution, context)?);
        } else if scan_optional_keyword_x(input, stores, execution, "depth")? {
            depth = Some(scan_scaled(input, stores, execution, context)?);
        } else {
            break;
        }
    }
    Ok(Node::Rule {
        width,
        height,
        depth,
    })
}

fn scan_hlist_group(
    input: &mut InputStack,
    stores: &mut Universe,
    execution: &mut crate::ExecutionContext<'_>,
    context: &'static str,
) -> Result<tex_state::ids::NodeListId, ExecError> {
    let opener =
        next_non_space_x(input, stores, execution)?.ok_or(ExecError::MissingToken { context })?;
    if !is_begin_group(opener) {
        return Err(ExecError::MissingToken { context });
    }
    stores.enter_group_with_kind(tex_state::GroupKind::Disc);
    let mut inner = ModeNest::new();
    inner.push(Mode::RestrictedHorizontal);
    let box_group_depth = stores.execution_group_depth();
    scan_box_group(&mut inner, input, stores, execution, box_group_depth)?;
    let level = inner.pop()?;
    let nodes = stores.freeze_node_list(level.list().nodes());
    crate::leave_group(input, stores, tex_state::GroupKind::Disc)?;
    Ok(nodes)
}

fn append_italic_correction(nest: &mut ModeNest, stores: &mut Universe) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores)?;
    let Some((font, ch)) = last_font_char(nest.current_list().nodes()) else {
        return Ok(());
    };
    let Ok(code) = font_code(ch) else {
        return Ok(());
    };
    let Some(metrics) = stores.font_char_metrics(font, code) else {
        return Ok(());
    };
    if metrics.italic_correction.raw() != 0 {
        nest.current_list_mut().push(Node::Kern {
            amount: metrics.italic_correction,
            kind: KernKind::Explicit,
        });
    }
    Ok(())
}

fn last_font_char(nodes: &[Node]) -> Option<(tex_state::ids::FontId, char)> {
    match nodes.last()? {
        Node::Char { font, ch, .. } | Node::Lig { font, ch, .. } => Some((*font, *ch)),
        _ => None,
    }
}

fn font_code(ch: char) -> Result<u8, ()> {
    u8::try_from(ch as u32).map_err(|_| ())
}

#[cfg(test)]
mod tests;
