use tex_fonts::{LigKernChar, LigKernCommand};
use tex_state::CommandContext;
use tex_state::diagnostic::DiagnosticEffects;
use tex_state::env::banks::{GlueParam, IntParam};
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::FontId;
use tex_state::math::{MathField, MathNoad, NoadClass, NoadKind};
use tex_state::meaning::UnexpandablePrimitive;
use tex_state::node::{BoxNode, BoxNodeFields, DiscKind, GlueKind, KernKind, Node, Sign};
use tex_state::scaled::{GlueSetRatio, Scaled};
use tex_state::token::OriginId;

use crate::mode::{PendingHChar, PendingHRunChar};
use crate::{ExecError, Mode, ModeNest};

pub(crate) fn append_character_with_fuel<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    ch: char,
    origin: OriginId,
    etex_extended: bool,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    debug_assert!(matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ));
    append_hchar_with_fuel(
        nest,
        stores,
        diagnostic_effects,
        ch,
        origin,
        etex_extended,
        fuel,
    )
}

/// Appends an ordinary space from main control after horizontal
/// mode has been selected by TeX82 §1095.
pub(crate) fn append_space_with_fuel<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    debug_assert!(matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ));
    flush_pending_hchars_with_fuel(nest, stores, diagnostic_effects, fuel)?;
    append_space_after_flush(nest, stores)
}

pub(crate) fn flush_pending_hchars<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let insert_hyphen_discs = nest.current_mode() == Mode::Horizontal;
    flush_pending_hchar_run_with_fuel(
        nest,
        stores,
        diagnostic_effects,
        insert_hyphen_discs,
        false,
        fuel,
    )
}

pub(crate) fn flush_pending_hchars_with_fuel<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores, diagnostic_effects, fuel)
}

/// Flushes the active TeX82 §1038 character run after its lookahead consumed
/// `\noboundary`. This suppresses only the right boundary; a separate flag on
/// the list records §1030's left-boundary cancellation before a new run.
pub(crate) fn flush_pending_hchars_without_right_boundary<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let insert_hyphen_discs = nest.current_mode() == Mode::Horizontal;
    flush_pending_hchar_run_with_fuel(
        nest,
        stores,
        diagnostic_effects,
        insert_hyphen_discs,
        true,
        fuel,
    )
}

/// Appends a whatsit node where tex.web §1356's `new_whatsit` would put it.
///
/// In tex.web an extension is reached through `main_control`'s `big_switch`,
/// which is where §1034's main loop exits once a non-character interrupts the
/// current word, so the characters built so far are already `tail` when
/// `new_whatsit` links itself on. Umber builds that word in one batch instead,
/// held in the list's pending run and appended only when something flushes
/// it -- so an unflushed whatsit is pushed *ahead* of every character of the
/// word it interrupts, and the shipped `.dvi` carries the `xxx1` before the
/// glyphs rather than between them (`umber2-alfh.22`).
///
/// Flushing here is also what keeps §1034's ligature and kerning boundaries
/// right: a whatsit ends the word, so the characters on either side of it
/// belong to two runs and must not ligature across it.
pub(crate) fn append_whatsit<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
    whatsit: tex_state::node::Whatsit,
) -> Result<(), ExecError> {
    flush_pending_hchars(nest, stores, diagnostic_effects, fuel)?;
    let contributes_directly_in_outer_vertical = matches!(
        whatsit,
        tex_state::node::Whatsit::OpenOut { .. }
            | tex_state::node::Whatsit::CloseOut { .. }
            | tex_state::node::Whatsit::DeferredWrite { .. }
            | tex_state::node::Whatsit::Special { .. }
            | tex_state::node::Whatsit::PdfReferenceObject { .. }
    );
    let node = Node::Whatsit(whatsit);
    if contributes_directly_in_outer_vertical {
        // TeX82 §1043 reaches the classic four extension subtypes through
        // `append_to_vlist` in outer vertical mode, where `tail` is the page
        // contribution list rather than the otherwise-empty mode list.
        // pdftex.web §1544's `pdf_ref_obj_node` has the same any-mode list
        // ownership, allowing §1054's end-job ejection to ship the reference.
        crate::vertical::append_vertical_contribution(nest, stores, node);
    } else {
        nest.current_list_mutation().push(node);
    }
    Ok(())
}

/// Closes the current list's mutable construction phase.
///
/// `ModeNest::pop` rejects a level that still owns a pending character run,
/// making this the only successful path from a live list to a packaged,
/// frozen, or otherwise detached list. Non-commit barriers can still call
/// [`flush_pending_hchars`] directly when TeX needs the run materialized but
/// must keep the mode level open.
pub(crate) fn commit_current_list<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
) -> Result<crate::mode::ModeLevelSummary, ExecError> {
    flush_pending_hchars(nest, stores, diagnostic_effects, fuel)?;
    nest.pop()
}

pub(crate) fn flush_pending_hchar_run_with_fuel<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    insert_hyphen_discs: bool,
    suppress_right_boundary: bool,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let Some(pending) = nest.current_list_mutation().take_pending_hchars() else {
        return Ok(());
    };
    if is_ltr_shaping_font(stores, pending.first.font) && is_supported_script(pending.script) {
        let language = nest.current_list().hyphen_language();
        let breaks = if insert_hyphen_discs {
            candidate_positions_for_chars(
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
        let mut list = nest.current_list_mutation();
        list.set_no_boundary(false);
        list.append(shaped);
        return Ok(());
    }
    let no_boundary = nest.current_list().no_boundary();
    let nodes = match run_tfm_ligature_machine(
        stores,
        diagnostic_effects,
        &pending.source,
        no_boundary,
        suppress_right_boundary,
        insert_hyphen_discs,
        fuel,
    ) {
        Ok(nodes) => nodes,
        Err(error) => {
            nest.current_list_mutation().set_pending_hchars(pending);
            return Err(ExecError::Command(error));
        }
    };
    let mut list = nest.current_list_mutation();
    list.set_no_boundary(false);
    list.append(nodes);
    Ok(())
}

fn candidate_positions_for_chars<G>(
    stores: &CommandContext<'_, G>,
    language: u8,
    chars: &[PendingHChar],
    left: usize,
    right: usize,
) -> Vec<usize> {
    if chars.len() > 63 || chars.len() < left.saturating_add(right) {
        return Vec::new();
    }
    let Some(first) = chars.first() else {
        return Vec::new();
    };
    if !(0..=255).contains(&stores.font_hyphen_char(first.font))
        || chars.iter().any(|entry| entry.font != first.font)
    {
        return Vec::new();
    }
    let Some(normalized) = chars
        .iter()
        .map(|entry| {
            stores
                .saved_hyphenation_code(language, entry.ch)
                .unwrap_or_else(|| {
                    char::from_u32(stores.lccode(entry.ch)).filter(|&mapped| mapped != '\0')
                })
        })
        .collect::<Option<String>>()
    else {
        return Vec::new();
    };
    if !normalized.starts_with(first.ch) && stores.int_param(IntParam::UC_HYPH) <= 0 {
        return Vec::new();
    }
    stores.hyphen_positions_for_language(language, &normalized, left, right)
}

pub(crate) fn append_space_after_flush<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
) -> Result<(), ExecError> {
    let configuration = stores.pdf_font_configuration();
    let sf = if configuration.adjusts_interword_glue() {
        1000
    } else {
        nest.current_list().space_factor()
    };
    let (mut spec, kind) = interword_glue(stores, sf);
    if configuration.adjusts_interword_glue() {
        adjust_interword_glue(stores, nest.current_list().nodes(), &mut spec);
    }
    nest.current_list_mutation().push(Node::Glue {
        spec,
        kind,
        leader: None,
    });
    Ok(())
}

/// Appends the explicit `\ ` control-space glue after horizontal mode has
/// already been selected. TeX82 §1030's `hmode+ex_space,mmode+ex_space: goto
/// append_normal_space` always takes the space-factor-1000 branch, unlike an
/// ordinary `spacer` token, which only reaches `append_normal_space` when
/// `space_factor=1000` and otherwise scales the glue through `app_space`
/// (§1042). Scanner fronts and main control share this typed
/// mode-switch-then-append operation.
pub(crate) fn append_control_space_glue_after_flush<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
) -> Result<(), ExecError> {
    let (mut spec, kind) = space_skip_or_font_space(stores, 1000);
    if stores.pdf_font_configuration().adjusts_interword_glue() {
        adjust_interword_glue(stores, nest.current_list().nodes(), &mut spec);
    }
    nest.current_list_mutation().push(Node::Glue {
        spec,
        kind,
        leader: None,
    });
    Ok(())
}

/// Appends the explicit `\ ` control-space glue from main control
/// after TeX82 §1090's vertical-mode paragraph start (if any) has already run.
/// Mirrors `append_canonical_space`'s split from `append_space` above.
pub(crate) fn append_control_space_with_fuel<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    debug_assert!(matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ));
    flush_pending_hchars_with_fuel(nest, stores, diagnostic_effects, fuel)?;
    append_control_space_glue_after_flush(nest, stores)
}

/// The `\ ` glue specification for TeX82 §1041's `append_normal_space` when
/// used from math mode (`mmode+ex_space`, §1030), which has no pending
/// ligature run or pdfTeX interword-glue adjustment to consider -- those are
/// exclusively horizontal-list concerns. Callers push the returned spec
/// directly onto the current (math) list.
pub(crate) fn control_space_glue_spec<G>(stores: &CommandContext<'_, G>) -> GlueSpec {
    space_skip_or_font_space(stores, 1000).0
}

pub(crate) fn append_hchar_with_fuel<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    ch: char,
    origin: OriginId,
    etex_extended: bool,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    let mode = nest.current_mode();
    fix_hyphen_language_with_fuel(nest, stores, diagnostic_effects, mode, fuel)?;
    let font = stores.current_font();
    let character_exists = stores.font_character_exists(font, ch);
    let font_is_ltr_shaping = stores.font_is_left_to_right_shaping(font);
    let false_boundary_character = font_code(ch)
        .ok()
        .is_some_and(|code| stores.font_false_boundary_char(font) == Some(code));
    if character_exists || false_boundary_character {
        let flush_incompatible_run = nest.current_list().pending_hchars().is_some_and(|pending| {
            (font_is_ltr_shaping
                || (pending.first.font != font && is_ltr_shaping_font(stores, pending.first.font)))
                && (pending.first.font != font
                    || !scripts_compatible(pending.script, tex_fonts::character_script(ch)))
        });
        if flush_incompatible_run {
            let insert_hyphen_discs = mode == Mode::Horizontal;
            flush_pending_hchar_run_with_fuel(
                nest,
                stores,
                diagnostic_effects,
                insert_hyphen_discs,
                false,
                fuel,
            )?;
        }
        let mut list = nest.current_list_mutation();
        append_pending_hchar(
            &mut list,
            stores,
            mode,
            font,
            font_is_ltr_shaping,
            ch,
            origin,
        );
        update_space_factor(&mut list, stores, ch);
        return Ok(());
    }
    flush_pending_hchar_run_with_fuel(
        nest,
        stores,
        diagnostic_effects,
        mode == Mode::Horizontal,
        false,
        fuel,
    )?;
    crate::diagnostics::report_missing_character_warning(
        stores,
        diagnostic_effects,
        font,
        ch,
        etex_extended,
    );
    Ok(())
}

/// TeX82 §1091's `norm_min`, verbatim: `if h<=0 then norm_min:=1 else if
/// h>=63 then norm_min:=63 else norm_min:=h`.
///
/// tex.web states this clamp once and applies it at every site that stores a
/// hyphen minimum in a fixed-width field: §1091's and §1200's `prev_graf`
/// packing, §1376's `fix_language`, and §1377's `\setlanguage`. It lives
/// here so all of them read the same function rather than each transcribing
/// the bounds.
pub(crate) const fn norm_min(value: i32) -> u8 {
    if value <= 0 {
        1
    } else if value >= 63 {
        63
    } else {
        value as u8
    }
}

/// TeX82 §§1091/1200's `set_cur_lang; clang:=cur_lang` paragraph-entry state.
///
/// The hyphen minima travel with `clang` in Umber's typed mode-list state so
/// the first §1376 `fix_language` node can retain the complete prior context.
pub(crate) fn current_hyphen_context<G>(stores: &CommandContext<'_, G>) -> (u8, u8, u8) {
    (
        u8::try_from(stores.int_param(IntParam::LANGUAGE)).unwrap_or(0),
        norm_min(stores.int_param(IntParam::LEFT_HYPHEN_MIN)),
        norm_min(stores.int_param(IntParam::RIGHT_HYPHEN_MIN)),
    )
}

pub(crate) fn indent_in_hmode<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    indent: bool,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    if !indent {
        return Ok(());
    }
    fn make_indent_box<G>(stores: &mut CommandContext<'_, G>) -> Node {
        let children = tex_state::node_arena::PageListId::empty();
        Node::HList(BoxNode::new(BoxNodeFields {
            width: stores.dimen_param(tex_state::env::banks::DimenParam::PAR_INDENT),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: tex_state::node::BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children,
        }))
    }
    if matches!(nest.current_mode(), Mode::Math | Mode::DisplayMath) {
        let indent_box = make_indent_box(stores);
        let list = stores.publish_page_nodes(vec![indent_box]);
        nest.current_list_mutation()
            .push(Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::SubBox(list),
            )));
    } else {
        flush_pending_hchars(nest, stores, diagnostic_effects, fuel)?;
        nest.current_list_mutation().set_space_factor(1000);
        let indent_box = make_indent_box(stores);
        nest.current_list_mutation().push(indent_box);
    }
    Ok(())
}

pub(crate) fn fix_hyphen_language_with_fuel<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    mode: Mode,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    if mode != Mode::Horizontal {
        return Ok(());
    }
    let (language, left_hyphen_min, right_hyphen_min) = current_hyphen_context(stores);
    if language == nest.current_list().hyphen_language() {
        return Ok(());
    }
    // tex.web's fix_language flushes the current ligature word before
    // recording the new language and its current hyphen minima. It does its
    // own flush rather than leaving it to `append_whatsit` because the caller
    // names the mode, so this run hyphenates even when the nest has already
    // moved on; `append_whatsit`'s flush then finds nothing pending.
    flush_pending_hchar_run_with_fuel(nest, stores, diagnostic_effects, true, false, fuel)?;
    append_whatsit(
        nest,
        stores,
        diagnostic_effects,
        fuel,
        tex_state::node::Whatsit::Language {
            language,
            left_hyphen_min,
            right_hyphen_min,
        },
    )?;
    nest.current_list_mutation()
        .set_hyphen_context(language, left_hyphen_min, right_hyphen_min);
    Ok(())
}

pub(crate) fn append_pending_hchar<G>(
    list: &mut crate::mode::ModeListMutation<'_>,
    _stores: &mut CommandContext<'_, G>,
    _mode: Mode,
    font: FontId,
    font_is_ltr_shaping: bool,
    ch: char,
    origin: OriginId,
) {
    let Some(mut pending) = list.take_pending_hchars() else {
        list.begin_pending_hchars(font, ch, origin);
        return;
    };
    if font_is_ltr_shaping
        && is_supported_script(pending.script)
        && is_supported_script(tex_fonts::character_script(ch))
    {
        let script = tex_fonts::character_script(ch);
        if is_strong_script(script) {
            pending.script = script;
        }
        pending.source.push(crate::mode::PendingHChar {
            font,
            ch,
            origin: origin.clone(),
        });
        pending.current = PendingHRunChar::new(font, ch, origin);
        list.set_pending_hchars(pending);
        return;
    }
    pending.source.push(crate::mode::PendingHChar {
        font,
        ch,
        origin: origin.clone(),
    });
    pending.current = PendingHRunChar::new(font, ch, origin);
    list.set_pending_hchars(pending);
}

pub(crate) fn is_strong_script(script: tex_fonts::Script) -> bool {
    !matches!(
        script,
        tex_fonts::Script::Common | tex_fonts::Script::Inherited | tex_fonts::Script::Unknown
    )
}

pub(crate) fn scripts_compatible(left: tex_fonts::Script, right: tex_fonts::Script) -> bool {
    !is_strong_script(left) || !is_strong_script(right) || left == right
}

pub(crate) fn is_supported_script(script: tex_fonts::Script) -> bool {
    matches!(
        script,
        tex_fonts::Script::Common
            | tex_fonts::Script::Inherited
            | tex_fonts::Script::Latin
            | tex_fonts::Script::Cyrillic
            | tex_fonts::Script::Greek
            | tex_fonts::Script::Han
            | tex_fonts::Script::Hiragana
            | tex_fonts::Script::Katakana
            | tex_fonts::Script::Hangul
            | tex_fonts::Script::Bopomofo
    )
}

pub(crate) fn is_ltr_shaping_font<G>(stores: &CommandContext<'_, G>, font: FontId) -> bool {
    stores.font_is_left_to_right_shaping(font)
}

pub(crate) fn shape_open_type_chars<G>(
    stores: &CommandContext<'_, G>,
    chars: &[crate::mode::PendingHChar],
    break_positions: &[usize],
) -> Vec<Node> {
    use std::collections::BTreeMap;

    let Some(first) = chars.first() else {
        return Vec::new();
    };
    let mut text = String::new();
    let mut byte_starts = Vec::with_capacity(chars.len());
    for entry in chars {
        byte_starts.push(text.len());
        if let Some(mapped) = stores.font_mapped_text(first.font, entry.ch) {
            text.push_str(mapped);
        } else {
            text.push(entry.ch);
        }
    }
    let break_bytes = break_positions
        .iter()
        .filter_map(|&position| byte_starts.get(position).copied())
        .collect::<Vec<_>>();
    let shaped = stores
        .shape_font_run(
            first.font,
            tex_fonts::ShapingRequest::with_breaks(&text, &break_bytes),
        )
        .expect("OpenType run font");
    let mut cluster_advances = BTreeMap::<usize, i64>::new();
    for glyph in shaped.glyphs {
        let cluster_byte = glyph.cluster as usize;
        let source_index = byte_starts
            .partition_point(|&start| start <= cluster_byte)
            .saturating_sub(1);
        *cluster_advances.entry(source_index).or_default() += i64::from(glyph.x_advance.raw());
    }
    let cluster_starts = cluster_advances.keys().copied().collect::<Vec<_>>();
    let mut adjustments = vec![Scaled::from_raw(0); chars.len()];
    for (cluster_index, &start) in cluster_starts.iter().enumerate() {
        let end = cluster_starts
            .get(cluster_index + 1)
            .copied()
            .unwrap_or(chars.len());
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
        let shaped = cluster_advances[&start];
        adjustments[end - 1] = Scaled::from_raw(
            i32::try_from(shaped - nominal).expect("shaped cluster adjustment fits Scaled"),
        );
    }
    let mut nodes = Vec::with_capacity(chars.len() * 2);
    for (entry, adjustment) in chars.iter().zip(adjustments) {
        nodes.push(Node::Char {
            font: entry.font,
            ch: entry.ch,
            origin: entry.origin.clone(),
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
pub(crate) fn reshape_open_type_runs<G>(stores: &CommandContext<'_, G>, nodes: &mut Vec<Node>) {
    let mut index = 0;
    while index < nodes.len() {
        let Node::Char { font, ch, origin } = &nodes[index] else {
            index += 1;
            continue;
        };
        if !is_ltr_shaping_font(stores, *font)
            || !is_supported_script(tex_fonts::character_script(*ch))
        {
            index += 1;
            continue;
        }
        let mut chars = vec![crate::mode::PendingHChar {
            font: *font,
            ch: *ch,
            origin: origin.clone(),
        }];
        let mut script = tex_fonts::character_script(*ch);
        let start = index;
        index += 1;
        while index < nodes.len() {
            match &nodes[index] {
                Node::Kern {
                    kind: KernKind::Font,
                    ..
                } => index += 1,
                Node::Char {
                    font: next_font,
                    ch: next_ch,
                    origin: next_origin,
                } if next_font == font
                    && scripts_compatible(script, tex_fonts::character_script(*next_ch)) =>
                {
                    let next_script = tex_fonts::character_script(*next_ch);
                    if is_strong_script(next_script) {
                        script = next_script;
                    }
                    chars.push(crate::mode::PendingHChar {
                        font: *font,
                        ch: *next_ch,
                        origin: next_origin.clone(),
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

pub(crate) fn reconstitute_with_fuel<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    pending: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
    insert_hyphen_discs: bool,
    fuel: &mut tex_command::CommandFuel,
) -> Result<Vec<Node>, tex_command::CommandError> {
    run_tfm_ligature_machine(
        stores,
        diagnostic_effects,
        pending,
        no_left_boundary,
        false,
        insert_hyphen_discs,
        fuel,
    )
}

#[derive(Clone)]
enum LigatureWorkItem {
    Boundary,
    Glyph(PendingHRunChar),
    Kern { amount: Scaled, kind: KernKind },
}

#[derive(Clone)]
struct LigatureWorkNode {
    item: LigatureWorkItem,
    previous: Option<usize>,
    next: Option<usize>,
    discard_if_missing: bool,
}

struct LigatureWorkList {
    nodes: Vec<LigatureWorkNode>,
    head: Option<usize>,
    tail: Option<usize>,
}

impl LigatureWorkList {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
            head: None,
            tail: None,
        }
    }

    fn push_back(&mut self, item: LigatureWorkItem) -> usize {
        let index = self.nodes.len();
        self.nodes.push(LigatureWorkNode {
            item,
            previous: self.tail,
            next: None,
            discard_if_missing: false,
        });
        if let Some(tail) = self.tail {
            self.nodes[tail].next = Some(index);
        } else {
            self.head = Some(index);
        }
        self.tail = Some(index);
        index
    }

    fn insert_after(&mut self, index: usize, item: LigatureWorkItem) -> usize {
        let next = self.nodes[index].next;
        let inserted = self.nodes.len();
        self.nodes.push(LigatureWorkNode {
            item,
            previous: Some(index),
            next,
            discard_if_missing: false,
        });
        self.nodes[index].next = Some(inserted);
        if let Some(next) = next {
            self.nodes[next].previous = Some(inserted);
        } else {
            self.tail = Some(inserted);
        }
        inserted
    }

    fn remove(&mut self, index: usize) {
        let previous = self.nodes[index].previous;
        let next = self.nodes[index].next;
        if let Some(previous) = previous {
            self.nodes[previous].next = next;
        } else {
            self.head = next;
        }
        if let Some(next) = next {
            self.nodes[next].previous = previous;
        } else {
            self.tail = previous;
        }
        self.nodes[index].previous = None;
        self.nodes[index].next = None;
    }
}

pub(crate) fn replacement_glyph(
    font: FontId,
    replacement: u8,
    consumed: impl IntoIterator<Item = PendingHRunChar>,
    left_hit: bool,
    right_hit: bool,
) -> PendingHRunChar {
    let mut orig = smallvec::SmallVec::new();
    let mut origins = smallvec::SmallVec::new();
    let mut inherited_left_hit = false;
    let mut inherited_right_hit = false;
    for glyph in consumed {
        inherited_left_hit |= glyph.left_hit;
        inherited_right_hit |= glyph.right_hit;
        orig.extend(glyph.orig);
        origins.extend(glyph.origins);
    }
    PendingHRunChar {
        font,
        ch: char::from(replacement),
        orig,
        origins,
        ligature_present: true,
        left_hit: left_hit || inherited_left_hit,
        right_hit: right_hit || inherited_right_hit,
    }
}

/// TeX82 §§1034-1036's complete ligature cursor.
///
/// Source glyphs, generated pseudo-ligatures, and both boundaries share one
/// work list. Thus every replacement pair re-enters the TFM program, and the
/// retain/delete and pass-over bits move one authoritative cursor.
pub(crate) fn run_tfm_ligature_machine<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    source: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
    suppress_right_boundary: bool,
    insert_hyphen_discs: bool,
    fuel: &mut tex_command::CommandFuel,
) -> Result<Vec<Node>, tex_command::CommandError> {
    let Some(first) = source.first() else {
        return Ok(Vec::new());
    };
    let font = first.font;
    let false_bchar = stores.font_false_boundary_char(font);
    let mut work = LigatureWorkList::with_capacity(source.len() + 4);
    if !no_left_boundary {
        work.push_back(LigatureWorkItem::Boundary);
    }
    for entry in source {
        work.push_back(LigatureWorkItem::Glyph(PendingHRunChar::new(
            entry.font,
            entry.ch,
            entry.origin.clone(),
        )));
    }
    if !suppress_right_boundary {
        work.push_back(LigatureWorkItem::Boundary);
    }

    let mut cursor = work.head;
    while let Some(left_index) = cursor {
        let Some(right_index) = work.nodes[left_index].next else {
            break;
        };
        fuel.charge()?;
        let left_item = work.nodes[left_index].item.clone();
        let right_item = work.nodes[right_index].item.clone();
        if matches!(left_item, LigatureWorkItem::Kern { .. })
            || matches!(right_item, LigatureWorkItem::Kern { .. })
        {
            cursor = Some(right_index);
            continue;
        }
        let pair: Option<(LigKernChar, LigKernChar)> = match (&left_item, &right_item) {
            (LigatureWorkItem::Boundary, LigatureWorkItem::Glyph(right)) => font_code(right.ch)
                .ok()
                .map(|right| (LigKernChar::Boundary, LigKernChar::Char(right))),
            (LigatureWorkItem::Glyph(left), LigatureWorkItem::Boundary) => font_code(left.ch)
                .ok()
                .map(|left| (LigKernChar::Char(left), LigKernChar::Boundary)),
            (LigatureWorkItem::Glyph(left), LigatureWorkItem::Glyph(right))
                if left.font == right.font =>
            {
                font_code(left.ch)
                    .ok()
                    .zip(font_code(right.ch).ok())
                    .map(|(left, right)| (LigKernChar::Char(left), LigKernChar::Char(right)))
            }
            _ => None,
        };
        let false_boundary_match = matches!(
            &right_item,
            LigatureWorkItem::Glyph(right)
                if right.font == font
                    && font_code(right.ch).ok().is_some_and(|code| Some(code) == false_bchar)
        );
        if false_boundary_match {
            work.nodes[right_index].discard_if_missing = true;
        }
        let Some((left_code, right_code)) = pair else {
            cursor = Some(right_index);
            continue;
        };

        if false_boundary_match {
            if let LigatureWorkItem::Glyph(right) = &right_item
                && !stores.font_character_exists(right.font, right.ch)
            {
                crate::diagnostics::report_missing_character_warning(
                    stores,
                    diagnostic_effects,
                    right.font,
                    right.ch,
                    false,
                );
                work.remove(right_index);
                break;
            }
            cursor = Some(right_index);
            continue;
        }

        let auto = match (&left_item, &right_item) {
            (LigatureWorkItem::Boundary, LigatureWorkItem::Glyph(right)) => {
                auto_kern(stores, right, Some(true))
            }
            (LigatureWorkItem::Glyph(left), LigatureWorkItem::Boundary) => {
                auto_kern(stores, left, None)
            }
            (LigatureWorkItem::Glyph(left), LigatureWorkItem::Glyph(right)) => {
                auto_kern_between(stores, left, right)
            }
            _ => None,
        };
        if let Some(Node::Kern { amount, kind }) = auto {
            let inserted = work.insert_after(left_index, LigatureWorkItem::Kern { amount, kind });
            cursor = work.nodes[inserted].next;
            continue;
        }

        let Some(command) = stores.font_lig_kern_command(font, left_code, right_code) else {
            cursor = Some(right_index);
            continue;
        };
        match command {
            LigKernCommand::Kern(amount) => {
                let inserted = work.insert_after(
                    left_index,
                    LigatureWorkItem::Kern {
                        amount,
                        kind: KernKind::Font,
                    },
                );
                cursor = work.nodes[inserted].next;
            }
            LigKernCommand::Ligature(lig) => {
                let consumed = [
                    lig.delete_current.then(|| work_glyph(&left_item)).flatten(),
                    lig.delete_next.then(|| work_glyph(&right_item)).flatten(),
                ]
                .into_iter()
                .flatten();
                let replacement = LigatureWorkItem::Glyph(replacement_glyph(
                    font,
                    lig.replacement,
                    consumed,
                    matches!(left_item, LigatureWorkItem::Boundary),
                    matches!(right_item, LigatureWorkItem::Boundary),
                ));
                match (lig.delete_current, lig.delete_next) {
                    (true, true) => {
                        work.nodes[left_index].item = replacement;
                        work.remove(right_index);
                    }
                    (true, false) => work.nodes[left_index].item = replacement,
                    (false, true) => work.nodes[right_index].item = replacement,
                    (false, false) => {
                        work.insert_after(left_index, replacement);
                    }
                }
                let op_byte = lig.pass_over * 4
                    + u8::from(!lig.delete_current) * 2
                    + u8::from(!lig.delete_next);
                cursor = Some(left_index);
                for _ in 0..match op_byte {
                    5..=7 => 1,
                    11 => 2,
                    _ => 0,
                } {
                    cursor = cursor.and_then(|index| work.nodes[index].next);
                }
            }
        }
    }

    let mut out = Vec::with_capacity(work.nodes.len() * 2);
    let mut pending_disc = None;
    let mut index = work.head;
    while let Some(current) = index {
        let item = work.nodes[current].item.clone();
        index = work.nodes[current].next;
        if !matches!(
            item,
            LigatureWorkItem::Kern {
                kind: KernKind::Auto,
                ..
            }
        ) {
            out.extend(pending_disc.take());
        }
        match item {
            LigatureWorkItem::Boundary => {}
            LigatureWorkItem::Glyph(glyph) => {
                if work.nodes[current].discard_if_missing
                    && !stores.font_character_exists(glyph.font, glyph.ch)
                {
                    crate::diagnostics::report_missing_character_warning(
                        stores,
                        diagnostic_effects,
                        glyph.font,
                        glyph.ch,
                        false,
                    );
                    continue;
                }
                let disc = literal_hyphen_disc(stores, &glyph, insert_hyphen_discs);
                out.push(rechar_node(glyph));
                pending_disc = disc;
            }
            LigatureWorkItem::Kern { amount, kind } => out.push(Node::Kern { amount, kind }),
        }
    }
    out.extend(pending_disc);
    Ok(out)
}

fn work_glyph(item: &LigatureWorkItem) -> Option<PendingHRunChar> {
    match item {
        LigatureWorkItem::Glyph(glyph) => Some(glyph.clone()),
        LigatureWorkItem::Boundary | LigatureWorkItem::Kern { .. } => None,
    }
}

pub(crate) fn auto_kern_between<G>(
    stores: &CommandContext<'_, G>,
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

pub(crate) fn auto_kern<G>(
    stores: &CommandContext<'_, G>,
    glyph: &PendingHRunChar,
    leading: Option<bool>,
) -> Option<Node> {
    match leading {
        Some(true) => auto_kern_codes(stores, glyph.font, None, Some(glyph.ch)),
        _ => auto_kern_codes(stores, glyph.font, Some(glyph.ch), None),
    }
}

pub(crate) fn auto_kern_codes<G>(
    stores: &CommandContext<'_, G>,
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

pub(crate) fn add_scaled(left: Scaled, right: Scaled) -> Scaled {
    left.checked_add(right)
        .expect("pdfTeX inter-character kern adjustment fits Scaled")
}

pub(crate) fn adjust_interword_glue<G>(
    stores: &CommandContext<'_, G>,
    nodes: &[Node],
    spec: &mut GlueSpec,
) {
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

pub(crate) fn scaled_font_code<G>(
    stores: &CommandContext<'_, G>,
    font: FontId,
    code: i32,
) -> Scaled {
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

pub(crate) fn rechar_node(current: PendingHRunChar) -> Node {
    if current.ligature_present {
        Node::Lig {
            font: current.font,
            ch: current.ch,
            orig: current.orig.into_vec(),
            origins: current.origins.into_vec(),
            left_hit: current.left_hit,
            right_hit: current.right_hit,
        }
    } else {
        Node::Char {
            font: current.font,
            ch: current.ch,
            origin: current
                .origins
                .first()
                .cloned()
                .unwrap_or(OriginId::UNKNOWN),
        }
    }
}

pub(crate) fn literal_hyphen_disc<G>(
    stores: &mut CommandContext<'_, G>,
    current: &PendingHRunChar,
    enabled: bool,
) -> Option<Node> {
    if !enabled
        || stores.font_hyphen_char(current.font)
            != current.orig.last().copied().unwrap_or(current.ch) as i32
    {
        return None;
    }
    let empty = tex_state::node_arena::PageListId::empty();
    Some(Node::Disc {
        kind: DiscKind::ExplicitHyphen,
        pre: empty.clone(),
        post: empty.clone(),
        replace: empty,
        physical_replace_count: 0,
    })
}

pub(crate) fn update_space_factor<G>(
    list: &mut crate::mode::ModeListMutation<'_>,
    stores: &CommandContext<'_, G>,
    ch: char,
) {
    list.set_space_factor(next_space_factor(list.space_factor(), stores, ch));
}

pub(crate) fn next_space_factor<G>(current: i32, stores: &CommandContext<'_, G>, ch: char) -> i32 {
    let sf = i32::from(stores.sfcode(ch));
    if sf == 0 {
        return current;
    }
    if sf > 1000 && current < 1000 {
        1000
    } else {
        sf
    }
}

pub(crate) fn interword_glue<G>(
    stores: &CommandContext<'_, G>,
    space_factor: i32,
) -> (GlueSpec, GlueKind) {
    let xspace = stores
        .glue_param(GlueParam::XSPACE_SKIP)
        .map_or(GlueSpec::ZERO, |id| stores.glue(id));
    if space_factor >= 2000 && xspace != GlueSpec::ZERO {
        // TeX82 §1042 appends a nonzero `\xspaceskip` verbatim.
        return (xspace, GlueKind::XSpaceSkip);
    }
    space_skip_or_font_space(stores, space_factor)
}

pub(crate) fn space_skip_or_font_space<G>(
    stores: &CommandContext<'_, G>,
    space_factor: i32,
) -> (GlueSpec, GlueKind) {
    let override_spec = stores
        .glue_param(GlueParam::SPACE_SKIP)
        .map_or(GlueSpec::ZERO, |id| stores.glue(id));
    if override_spec != GlueSpec::ZERO {
        // TeX82 §1042 scales nonzero `\spaceskip` through `app_space`.
        let mut spec = override_spec;
        if space_factor != 1000 {
            spec.stretch = scale_by_factor(spec.stretch, space_factor, 1000);
            spec.shrink = scale_by_factor(spec.shrink, 1000, space_factor);
        }
        return (spec, GlueKind::SpaceSkip);
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
    (spec, GlueKind::Normal)
}

pub(crate) fn scale_by_factor(value: Scaled, num: i32, den: i32) -> Scaled {
    Scaled::from_raw(((i64::from(value.raw()) * i64::from(num)) / i64::from(den)) as i32)
}

pub(crate) fn infinite_glue(order: Order, negative: bool, shrink: bool) -> GlueSpec {
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

pub(crate) fn append_italic_correction_with_fuel<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    fuel: &mut tex_command::CommandFuel,
) -> Result<(), ExecError> {
    flush_pending_hchars_with_fuel(nest, stores, diagnostic_effects, fuel)?;
    let Some((font, ch)) = last_font_char(nest.current_list().nodes()) else {
        return Ok(());
    };
    let Ok(code) = font_code(ch) else {
        return Ok(());
    };
    let Some(metrics) = stores.font_char_metrics(font, code) else {
        return Ok(());
    };
    nest.current_list_mutation().push(Node::Kern {
        amount: metrics.italic_correction,
        kind: KernKind::Explicit,
    });
    Ok(())
}

pub(crate) fn last_font_char(nodes: &[Node]) -> Option<(tex_state::ids::FontId, char)> {
    match nodes.last()? {
        Node::Char { font, ch, .. } | Node::Lig { font, ch, .. } => Some((*font, *ch)),
        _ => None,
    }
}

pub(crate) fn font_code(ch: char) -> Result<u8, ()> {
    u8::try_from(ch as u32).map_err(|_| ())
}
