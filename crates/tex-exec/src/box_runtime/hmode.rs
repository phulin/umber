use smallvec::SmallVec;
use tex_fonts::{CharMetrics, LigKernChar, LigKernCommand};
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

/// Result of admitting one borrowed source-character prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CharacterRunAppend {
    pub(crate) count: u32,
    pub(crate) continue_run: bool,
}

#[inline(always)]
#[allow(clippy::too_many_arguments)] // Candidate classification keeps source, font, and list state explicit.
fn classify_character_run_candidate<G>(
    run: &tex_command::BorrowedSourceCharacterRun<'_>,
    index: usize,
    font: FontId,
    font_is_ltr_shaping: bool,
    false_boundary: Option<u8>,
    stores: &CommandContext<'_, G>,
    pending_script: &mut Option<tex_fonts::Script>,
    continue_run: &mut bool,
    space_factor: &mut i32,
) -> Option<(char, OriginId, Option<tex_fonts::Script>, tex_fonts::Script)> {
    if !*continue_run {
        return None;
    }
    let byte = *run.bytes().get(index)?;
    if !byte.is_ascii() {
        *continue_run = false;
        return None;
    }
    let ch = char::from(byte);
    let script = tex_fonts::character_script(ch);
    if index != 0
        && font_is_ltr_shaping
        && pending_script.is_some_and(|pending| {
            is_supported_script(pending)
                && is_supported_script(script)
                && is_strong_script(script)
                && !scripts_compatible(pending, script)
        })
    {
        *continue_run = false;
        return None;
    }
    let is_false_boundary = false_boundary == Some(byte);
    let has_metrics = stores.font_character_metrics(font, ch).is_some();
    if !has_metrics && !is_false_boundary {
        *continue_run = false;
        return None;
    }
    if is_false_boundary && !has_metrics {
        *continue_run = false;
    }
    let script_option = pending_script.and_then(|pending| {
        (font_is_ltr_shaping
            && is_supported_script(pending)
            && is_supported_script(script)
            && is_strong_script(script))
        .then_some(script)
    });
    if pending_script.is_none() || script_option.is_some() {
        *pending_script = Some(script);
    }
    let origin = run.origin(index);
    *space_factor = next_space_factor(*space_factor, stores, ch);
    Some((ch, origin, script_option, script))
}

/// Preflights and appends one borrowed ordinary source run.
///
/// Font and pending-run compatibility are read once for the prefix.  The
/// source bytes remain borrowed throughout admission; only the existing
/// `PendingHRun` vector receives semantic characters. The processor charges
/// fuel once for the admitted prefix after this callback returns.
pub(crate) fn append_character_run_with_fuel<G>(
    nest: &mut ModeNest,
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    run: tex_command::BorrowedSourceCharacterRun<'_>,
    _etex_extended: bool,
    fuel: &mut tex_command::CommandFuel,
) -> Result<CharacterRunAppend, ExecError> {
    debug_assert!(matches!(
        nest.current_mode(),
        Mode::Horizontal | Mode::RestrictedHorizontal
    ));
    let mode = nest.current_mode();
    let font = stores.current_font();
    let font_is_ltr_shaping = stores.font_is_left_to_right_shaping(font);
    let false_boundary = stores.font_false_boundary_char(font);
    let language_changed = mode == Mode::Horizontal
        && current_hyphen_context(stores).0 != nest.current_list().hyphen_language();
    let mut pending_script = if language_changed {
        None
    } else {
        nest.current_list()
            .pending_hchars()
            .map(|pending| pending.script)
    };
    if fuel.remaining() == 0 {
        fuel.charge().map_err(ExecError::Command)?;
        return Ok(CharacterRunAppend {
            count: 0,
            continue_run: false,
        });
    }
    let mut space_factor = nest.current_list().space_factor();
    let mut continue_run = true;
    let first = classify_character_run_candidate(
        &run,
        0,
        font,
        font_is_ltr_shaping,
        false_boundary,
        stores,
        &mut pending_script,
        &mut continue_run,
        &mut space_factor,
    );
    let Some((first, first_origin, mut first_script_option, first_script)) = first else {
        return Ok(CharacterRunAppend {
            count: 0,
            continue_run: false,
        });
    };
    let pending_incompatible = nest.current_list().pending_hchars().is_some_and(|pending| {
        let pending_font = pending.source[0].font;
        let pending_ltr = pending_font != font && is_ltr_shaping_font(stores, pending_font);
        (font_is_ltr_shaping || pending_ltr)
            && (pending_font != font || !scripts_compatible(pending.script, first_script))
    });
    if pending_incompatible {
        pending_script = None;
        first_script_option = None;
    }
    fix_hyphen_language_with_fuel(nest, stores, diagnostic_effects, mode, fuel)?;
    if pending_incompatible && !language_changed {
        flush_pending_hchar_run_with_fuel(
            nest,
            stores,
            diagnostic_effects,
            mode == Mode::Horizontal,
            false,
            fuel,
        )?;
    }
    let available = usize::try_from(fuel.remaining()).unwrap_or(usize::MAX);
    let capacity = run.bytes().len().min(available);
    if capacity == 0 {
        fuel.charge().map_err(ExecError::Command)?;
        return Ok(CharacterRunAppend {
            count: 0,
            continue_run: false,
        });
    }
    let mut first = Some((first, first_origin, first_script_option));
    let accepted = {
        let mut list = nest.current_list_mutation();
        let accepted = list.append_pending_hchars(font, capacity, |index| {
            if index == 0 {
                first.take()
            } else {
                classify_character_run_candidate(
                    &run,
                    index,
                    font,
                    font_is_ltr_shaping,
                    false_boundary,
                    stores,
                    &mut pending_script,
                    &mut continue_run,
                    &mut space_factor,
                )
                .map(|(ch, origin, script_option, _)| (ch, origin, script_option))
            }
        });
        if accepted != 0 {
            list.set_space_factor(space_factor);
        }
        accepted
    };
    let accepted_u32 = u32::try_from(accepted).expect("source run length fits packed u32");
    Ok(CharacterRunAppend {
        count: accepted_u32,
        continue_run: continue_run
            && accepted == capacity
            && capacity == run.bytes().len()
            && !run.fuel_limited(),
    })
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
    // TeX82 §1043 reaches classic extensions through the current-list tail,
    // while pdftex.web §§1524/1563--1567 add further any-mode whatsits at the
    // same list boundary. In Umber the outer vertical current list is the page
    // contribution queue; internal vertical, box-building, and math lists are
    // still owned by their `ModeList`. Route every whatsit through the shared
    // ownership decision so a new subtype cannot accidentally retain an old
    // page-region handle across output succession.
    crate::vertical::append_node_to_current_list(nest, stores, Node::Whatsit(whatsit))
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
    let Some(pending) = nest.current_list().pending_hchars() else {
        return Ok(());
    };
    let first_font = pending
        .source
        .first()
        .expect("a pending horizontal run owns its first source character")
        .font;
    let use_open_type =
        is_ltr_shaping_font(stores, first_font) && is_supported_script(pending.script);
    let language = nest.current_list().hyphen_language();
    let no_boundary = nest.current_list().no_boundary();
    let result = {
        let mut list = nest.current_list_mutation();
        list.append_pending_constructed(stores, |stores, source, shaping, tfm_work, output| {
            let mut sink = PageNodeSink { output };
            if use_open_type {
                if insert_hyphen_discs {
                    prepare_candidate_positions_for_chars(
                        stores,
                        language,
                        source,
                        stores.int_param(IntParam::LEFT_HYPHEN_MIN).max(1) as usize,
                        stores.int_param(IntParam::RIGHT_HYPHEN_MIN).max(1) as usize,
                        shaping,
                    );
                }
                shape_open_type_chars_into(stores, source, shaping, &mut sink);
                Ok(())
            } else {
                run_tfm_ligature_machine_with_work(
                    stores,
                    diagnostic_effects,
                    source,
                    no_boundary,
                    if suppress_right_boundary {
                        LigatureRightBoundary::Suppressed
                    } else {
                        LigatureRightBoundary::Font
                    },
                    insert_hyphen_discs,
                    fuel,
                    tfm_work,
                    &mut sink,
                )
                .map_err(ExecError::Command)
            }
        })
    };
    result?;
    let mut list = nest.current_list_mutation();
    assert!(list.clear_pending_hchars());
    list.set_no_boundary(false);
    Ok(())
}

fn prepare_candidate_positions_for_chars<G>(
    stores: &CommandContext<'_, G>,
    language: u8,
    chars: &[PendingHChar],
    left: usize,
    right: usize,
    scratch: &mut OpenTypeShapingScratch,
) {
    // The pending-run owner supplies all of these buffers.  Clearing them at
    // entry also makes every early return leave no logical candidate state
    // behind for a later flush.
    scratch.candidate_positions.clear();
    scratch.hyphenation_text.clear();
    if chars.len() > 63 || chars.len() < left.saturating_add(right) {
        return;
    }
    let Some(first) = chars.first() else {
        return;
    };
    if !(0..=255).contains(&stores.font_hyphen_char(first.font))
        || chars.iter().any(|entry| entry.font != first.font)
    {
        return;
    }
    for entry in chars {
        let Some(mapped) = stores
            .saved_hyphenation_code(language, entry.ch)
            .unwrap_or_else(|| {
                char::from_u32(stores.lccode(entry.ch)).filter(|&mapped| mapped != '\0')
            })
        else {
            return;
        };
        scratch.hyphenation_text.push(mapped);
    }
    if !scratch.hyphenation_text.starts_with(first.ch) && stores.int_param(IntParam::UC_HYPH) <= 0 {
        return;
    }
    stores.hyphen_positions_for_language_into(
        language,
        &scratch.hyphenation_text,
        left,
        right,
        &mut scratch.candidate_positions,
        &mut scratch.hyphenation,
    );
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
        adjust_interword_glue(stores, nest.current_list().nodes(stores), &mut spec);
    }
    nest.current_list_mutation().push(
        stores,
        Node::Glue {
            spec,
            kind,
            leader: None,
        },
    );
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
        adjust_interword_glue(stores, nest.current_list().nodes(stores), &mut spec);
    }
    nest.current_list_mutation().push(
        stores,
        Node::Glue {
            spec,
            kind,
            leader: None,
        },
    );
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
                || (pending.source[0].font != font
                    && is_ltr_shaping_font(stores, pending.source[0].font)))
                && (pending.source[0].font != font
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
        nest.current_list_mutation().push(
            stores,
            Node::MathNoad(MathNoad::new(
                NoadKind::Normal(NoadClass::Ord),
                MathField::SubBox(list),
            )),
        );
    } else {
        flush_pending_hchars(nest, stores, diagnostic_effects, fuel)?;
        nest.current_list_mutation().set_space_factor(1000);
        let indent_box = make_indent_box(stores);
        nest.current_list_mutation().push(stores, indent_box);
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
    let script = list.pending_hchars().and_then(|pending| {
        let script = tex_fonts::character_script(ch);
        (font_is_ltr_shaping
            && is_supported_script(pending.script)
            && is_supported_script(script)
            && is_strong_script(script))
        .then_some(script)
    });
    let appended = list.append_pending_hchar(font, ch, origin, script);
    if !appended {
        list.begin_pending_hchars(font, ch, origin);
    }
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

/// Final nodes emitted by shaping and reconstitution.
///
/// The sink deliberately carries no `Node` value.  Production sinks reserve
/// and initialize the final page-arena slot immediately. Hyphenation cursors
/// use the same sink contract while replaying child lists against their
/// move-only TFM tape.
pub(crate) trait FinalHNodeSink {
    fn glyph<G>(&mut self, stores: &mut CommandContext<'_, G>, glyph: PendingHRunChar);

    /// Consumes one move-only TFM cell while it is still attached to the
    /// reconstitution tape.  The default path writes the resident record
    /// directly from the borrowed provenance span.  A streaming hyphenation
    /// cursor can override this hook to suspend the output at a source
    /// boundary and replay a child list against the same tape.
    fn glyph_cell<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        glyph: LigatureGlyphCell,
        source: &[LigatureSourceEntry],
    ) {
        self.glyph(stores, glyph.into_pending(source));
    }

    fn glyph_cell_with_work<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        _current: usize,
        glyph: LigatureGlyphCell,
        work: &mut LigatureWorkList,
        _diagnostic_effects: &mut DiagnosticEffects,
        _fuel: &mut tex_command::CommandFuel,
    ) -> Result<(), tex_command::CommandError> {
        let source = work.source(glyph.provenance);
        self.glyph_cell(stores, glyph, source);
        Ok(())
    }

    fn kern<G>(&mut self, stores: &mut CommandContext<'_, G>, amount: Scaled, kind: KernKind);

    #[allow(clippy::too_many_arguments)] // The callback carries the live TFM cell and diagnostic/fuel state.
    fn kern_with_work<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        _current: usize,
        amount: Scaled,
        kind: KernKind,
        _work: &mut LigatureWorkList,
        _diagnostic_effects: &mut DiagnosticEffects,
        _fuel: &mut tex_command::CommandFuel,
    ) -> Result<(), tex_command::CommandError> {
        self.kern(stores, amount, kind);
        Ok(())
    }

    fn explicit_hyphen_disc<G>(&mut self, stores: &mut CommandContext<'_, G>);

    fn discretionary<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        kind: DiscKind,
        pre: tex_state::node_arena::PageListId,
        post: tex_state::node_arena::PageListId,
        replace: tex_state::node_arena::PageListId,
        physical_replace_count: u8,
    );
}

pub(crate) struct PageNodeSink<'a> {
    pub(crate) output: &'a mut tex_state::page_node_arena::PageMaterialActiveListBuilder,
}

impl FinalHNodeSink for PageNodeSink<'_> {
    fn glyph<G>(&mut self, stores: &mut CommandContext<'_, G>, glyph: PendingHRunChar) {
        stores.construct_page_active_list(self.output, |destination| {
            if glyph.ligature_present {
                let origins_empty = glyph.origins.is_empty();
                destination.ligature_iter_with_origin_mode(
                    glyph.font,
                    glyph.ch,
                    glyph.left_hit,
                    glyph.right_hit,
                    origins_empty,
                    glyph.orig.iter().enumerate().map(|(index, ch)| {
                        (
                            *ch,
                            if origins_empty {
                                OriginId::UNKNOWN
                            } else {
                                glyph.origins[index]
                            },
                        )
                    }),
                );
            } else {
                destination.char(
                    glyph.font,
                    glyph.ch,
                    glyph.origins.first().cloned().unwrap_or(OriginId::UNKNOWN),
                );
            }
        });
    }

    fn glyph_cell<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        glyph: LigatureGlyphCell,
        source: &[LigatureSourceEntry],
    ) {
        stores.construct_page_active_list(self.output, |destination| {
            if glyph.ligature_present {
                destination.ligature_iter(
                    glyph.font,
                    glyph.ch,
                    glyph.left_hit,
                    glyph.right_hit,
                    source.iter().map(|entry| (entry.ch, entry.origin)),
                );
            } else {
                destination.char(
                    glyph.font,
                    glyph.ch,
                    source
                        .first()
                        .map_or(OriginId::UNKNOWN, |entry| entry.origin),
                );
            }
        });
    }

    fn kern<G>(&mut self, stores: &mut CommandContext<'_, G>, amount: Scaled, kind: KernKind) {
        stores.construct_page_active_list(self.output, |destination| {
            destination.kern(amount, kind);
        });
    }

    fn explicit_hyphen_disc<G>(&mut self, stores: &mut CommandContext<'_, G>) {
        let empty = tex_state::node_arena::PageListId::empty();
        self.discretionary(stores, DiscKind::ExplicitHyphen, empty, empty, empty, 0);
    }

    fn discretionary<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        kind: DiscKind,
        pre: tex_state::node_arena::PageListId,
        post: tex_state::node_arena::PageListId,
        replace: tex_state::node_arena::PageListId,
        physical_replace_count: u8,
    ) {
        stores.construct_page_active_list(self.output, |destination| {
            destination.discretionary(kind, pre, post, replace, physical_replace_count);
        });
    }
}

fn shape_open_type_chars_into<G>(
    stores: &mut CommandContext<'_, G>,
    chars: &[crate::mode::PendingHChar],
    scratch: &mut OpenTypeShapingScratch,
    sink: &mut impl FinalHNodeSink,
) {
    plan_open_type_adjustments(&*stores, chars, scratch);
    for (index, entry) in chars.iter().enumerate() {
        let adjustment = scratch.adjustments[index];
        sink.glyph(
            stores,
            PendingHRunChar::new(entry.font, entry.ch, entry.origin),
        );
        if adjustment.raw() != 0 {
            sink.kern(stores, adjustment, KernKind::Font);
        }
    }
    scratch.clear();
}

#[derive(Default)]
pub(crate) struct HorizontalFlushScratch {
    source_chars: Vec<crate::mode::PendingHChar>,
    text: String,
    byte_starts: Vec<usize>,
    break_bytes: Vec<usize>,
    candidate_positions: Vec<usize>,
    hyphenation_text: String,
    hyphenation: tex_state::hyphenation::HyphenationScratch,
    nominal_widths: Vec<i64>,
    cluster_accum: Vec<i64>,
    cluster_seen: Vec<bool>,
    cluster_advances: Vec<(usize, i64)>,
    adjustments: Vec<Scaled>,
    shaping: tex_fonts::ShapingScratch,
}

/// Compatibility name retained for the existing hmode callers and tests.
pub(crate) type OpenTypeShapingScratch = HorizontalFlushScratch;

impl HorizontalFlushScratch {
    pub(crate) fn clear(&mut self) {
        self.source_chars.clear();
        self.text.clear();
        self.byte_starts.clear();
        self.break_bytes.clear();
        self.candidate_positions.clear();
        self.hyphenation_text.clear();
        self.hyphenation.clear();
        self.nominal_widths.clear();
        self.cluster_accum.clear();
        self.cluster_seen.clear();
        self.cluster_advances.clear();
        self.adjustments.clear();
        self.shaping.clear();
    }
}

fn plan_open_type_adjustments<G>(
    stores: &CommandContext<'_, G>,
    chars: &[crate::mode::PendingHChar],
    scratch: &mut OpenTypeShapingScratch,
) {
    scratch.text.clear();
    scratch.byte_starts.clear();
    scratch.break_bytes.clear();
    scratch.nominal_widths.clear();
    scratch.cluster_accum.clear();
    scratch.cluster_seen.clear();
    scratch.cluster_advances.clear();
    scratch.adjustments.clear();
    let Some(first) = chars.first() else {
        return;
    };
    scratch.byte_starts.reserve(chars.len());
    scratch.nominal_widths.reserve(chars.len());
    let mut candidate_index = 0usize;
    for (source_index, entry) in chars.iter().enumerate() {
        while scratch.candidate_positions.get(candidate_index) == Some(&source_index) {
            scratch.break_bytes.push(scratch.text.len());
            candidate_index += 1;
        }
        scratch.byte_starts.push(scratch.text.len());
        if let Some(mapped) = stores.font_mapped_text(entry.font, entry.ch) {
            scratch.text.push_str(mapped);
        } else {
            scratch.text.push(entry.ch);
        }
        scratch.nominal_widths.push(i64::from(
            stores
                .font_character_metrics(entry.font, entry.ch)
                .map_or(0, |metrics| metrics.width.raw()),
        ));
    }
    scratch.adjustments.resize(chars.len(), Scaled::from_raw(0));
    scratch.cluster_accum.resize(chars.len(), 0);
    scratch.cluster_seen.resize(chars.len(), false);
    scratch.cluster_accum.fill(0);
    scratch.cluster_seen.fill(false);

    let mut source_cursor = 0usize;
    let mut last_cluster = None;
    let mut fallback = false;
    let byte_starts = &scratch.byte_starts;
    let cluster_accum = &mut scratch.cluster_accum;
    let cluster_seen = &mut scratch.cluster_seen;
    let cluster_advances = &mut scratch.cluster_advances;
    let _metadata = stores
        .shape_font_run_with_scratch(
            first.font,
            tex_fonts::ShapingRequest::with_breaks(&scratch.text, &scratch.break_bytes),
            &mut scratch.shaping,
            |glyph| {
                let cluster_byte = glyph.cluster as usize;
                if !fallback {
                    if last_cluster.is_some_and(|previous| cluster_byte < previous) {
                        fallback = true;
                        for (index, seen) in cluster_seen.iter().copied().enumerate() {
                            if seen {
                                cluster_advances.push((index, cluster_accum[index]));
                            }
                        }
                    } else {
                        last_cluster = Some(cluster_byte);
                        while source_cursor + 1 < byte_starts.len()
                            && byte_starts[source_cursor + 1] <= cluster_byte
                        {
                            source_cursor += 1;
                        }
                        let source_index = source_cursor.min(byte_starts.len() - 1);
                        cluster_accum[source_index] += i64::from(glyph.x_advance.raw());
                        cluster_seen[source_index] = true;
                        return;
                    }
                }
                let source_index = byte_starts
                    .partition_point(|&start| start <= cluster_byte)
                    .saturating_sub(1);
                cluster_advances.push((source_index, i64::from(glyph.x_advance.raw())));
            },
        )
        .expect("OpenType run font");
    if fallback {
        scratch
            .cluster_advances
            .sort_unstable_by_key(|entry| entry.0);
        let mut cluster = 0usize;
        while cluster < scratch.cluster_advances.len() {
            let start = scratch.cluster_advances[cluster].0;
            let mut shaped = 0_i64;
            let mut next = cluster;
            while next < scratch.cluster_advances.len() && scratch.cluster_advances[next].0 == start
            {
                shaped += scratch.cluster_advances[next].1;
                next += 1;
            }
            let end = scratch
                .cluster_advances
                .get(next)
                .map_or(chars.len(), |entry| entry.0);
            finish_open_type_cluster(
                &mut scratch.adjustments,
                &scratch.nominal_widths,
                start,
                end,
                shaped,
            );
            cluster = next;
        }
    } else {
        let mut cluster_start = None;
        let mut shaped = 0_i64;
        for (source_index, seen) in scratch.cluster_seen.iter().copied().enumerate() {
            if seen {
                if let Some(start) = cluster_start {
                    finish_open_type_cluster(
                        &mut scratch.adjustments,
                        &scratch.nominal_widths,
                        start,
                        source_index,
                        shaped,
                    );
                }
                cluster_start = Some(source_index);
                shaped = scratch.cluster_accum[source_index];
            }
        }
        if let Some(start) = cluster_start {
            finish_open_type_cluster(
                &mut scratch.adjustments,
                &scratch.nominal_widths,
                start,
                chars.len(),
                shaped,
            );
        }
    }
}

fn finish_open_type_cluster(
    adjustments: &mut [Scaled],
    nominal_widths: &[i64],
    start: usize,
    end: usize,
    shaped: i64,
) {
    if start >= end {
        return;
    }
    let nominal = nominal_widths[start..end].iter().copied().sum::<i64>();
    adjustments[end - 1] = Scaled::from_raw(
        i32::try_from(shaped - nominal).expect("shaped cluster adjustment fits Scaled"),
    );
}

/// Replaces provisional OpenType adjustments while retaining every unchanged
/// page-material span by coordinate.
#[derive(Clone, Copy)]
enum OpenTypeSourceNode {
    Character {
        font: FontId,
        ch: char,
        origin: OriginId,
    },
    FontKern,
    Other,
}

impl OpenTypeSourceNode {
    fn from_node(node: tex_state::page_node_arena::PageMaterialNodeRef<'_>) -> Self {
        if let Some((font, ch, origin)) = node.character() {
            Self::Character { font, ch, origin }
        } else if node.is_font_kern() {
            Self::FontKern
        } else {
            Self::Other
        }
    }
}

struct OpenTypeSourceWalk<'a> {
    output: &'a mut tex_state::page_node_arena::PageMaterialActiveListBuilder,
    shaping: &'a mut OpenTypeShapingScratch,
    saw_run: bool,
    retained_start: usize,
    run_font: Option<FontId>,
    run_script: tex_fonts::Script,
}

impl OpenTypeSourceWalk<'_> {
    fn visit_chunk_prefix<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        source: tex_state::page_node_arena::PageListSpan,
        mut chunk: tex_state::page_node_arena::PageListChunkCursor,
    ) {
        if let Some(previous) = stores
            .page_node_span_previous_chunk(&chunk)
            .expect("OpenType source chunk remains live")
        {
            self.visit_chunk_prefix(stores, source, previous);
        }
        while let Some((index, node)) = stores.page_node_span_next_chunk_node(&mut chunk) {
            let observed = { OpenTypeSourceNode::from_node(node) };
            self.visit_node(stores, source, index, observed);
        }
    }

    fn visit_node<G>(
        &mut self,
        stores: &mut CommandContext<'_, G>,
        source: tex_state::page_node_arena::PageListSpan,
        index: usize,
        observed: OpenTypeSourceNode,
    ) {
        if let Some(font) = self.run_font {
            match observed {
                OpenTypeSourceNode::FontKern => return,
                OpenTypeSourceNode::Character {
                    font: next_font,
                    ch,
                    origin,
                } if next_font == font
                    && scripts_compatible(self.run_script, tex_fonts::character_script(ch)) =>
                {
                    let next_script = tex_fonts::character_script(ch);
                    if is_strong_script(next_script) {
                        self.run_script = next_script;
                    }
                    self.shaping
                        .source_chars
                        .push(crate::mode::PendingHChar { font, ch, origin });
                    return;
                }
                _ => self.flush_run(stores, index),
            }
        }

        if let OpenTypeSourceNode::Character { font, ch, origin } = observed
            && is_ltr_shaping_font(stores, font)
            && is_supported_script(tex_fonts::character_script(ch))
        {
            if !self.saw_run {
                stores.open_page_active_list(self.output);
                self.saw_run = true;
            }
            if self.retained_start < index {
                stores.append_page_active_span_range(
                    self.output,
                    source,
                    self.retained_start..index,
                );
            }
            self.shaping.source_chars.clear();
            self.shaping
                .source_chars
                .push(crate::mode::PendingHChar { font, ch, origin });
            self.run_font = Some(font);
            self.run_script = tex_fonts::character_script(ch);
        }
    }

    fn flush_run<G>(&mut self, stores: &mut CommandContext<'_, G>, end: usize) {
        let chars = std::mem::take(&mut self.shaping.source_chars);
        {
            plan_open_type_adjustments(stores, &chars, self.shaping);
            for (index, entry) in chars.iter().enumerate() {
                let adjustment = self.shaping.adjustments[index];
                stores.construct_page_active_list(self.output, |destination| {
                    destination.char(entry.font, entry.ch, entry.origin);
                });
                if adjustment.raw() != 0 {
                    stores.construct_page_active_list(self.output, |destination| {
                        destination.kern(adjustment, KernKind::Font);
                    });
                }
            }
        }
        self.shaping.source_chars = chars;
        self.shaping.clear();
        self.run_font = None;
        self.retained_start = end;
    }
}

pub(crate) fn reshape_open_type_runs_list<G>(
    stores: &mut CommandContext<'_, G>,
    source: tex_state::node_arena::PageListId,
    shaping: &mut OpenTypeShapingScratch,
) -> tex_state::node_arena::PageListId {
    shaping.clear();
    let source = stores
        .admit_page_node_span(source)
        .expect("OpenType source crosses one live page-region boundary");
    let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
    let mut walk = OpenTypeSourceWalk {
        output: &mut output,
        shaping,
        saw_run: false,
        retained_start: 0,
        run_font: None,
        run_script: tex_fonts::Script::Common,
    };
    if let Some(tail) = stores
        .page_node_span_tail_chunk(source)
        .expect("OpenType source remains admitted")
    {
        walk.visit_chunk_prefix(stores, source, tail);
    }
    if walk.run_font.is_some() {
        walk.flush_run(stores, source.len());
    } else if walk.saw_run && walk.retained_start < source.len() {
        stores.append_page_active_span_range(
            walk.output,
            source,
            walk.retained_start..source.len(),
        );
    }
    let saw_run = walk.saw_run;
    shaping.clear();
    if saw_run {
        stores.finalize_page_active_list(&mut output)
    } else {
        source.list()
    }
}

#[derive(Clone, Copy)]
pub(crate) enum LigatureRightBoundary {
    Suppressed,
    Font,
    Character(u8),
}

#[derive(Clone, Copy)]
pub(crate) struct LigatureSourceEntry {
    pub(crate) ch: char,
    pub(crate) origin: OriginId,
}

#[derive(Clone, Copy)]
pub(crate) struct ProvenanceSpan {
    pub(crate) start: usize,
    pub(crate) len: usize,
}

impl ProvenanceSpan {
    const EMPTY: Self = Self { start: 0, len: 0 };

    pub(crate) fn end(self) -> usize {
        self.start + self.len
    }
}

#[derive(Clone, Copy)]
struct LigatureBoundaryCell {
    code: Option<u8>,
    lig_kern_start: Option<u16>,
    leading_auto_kern: Scaled,
}

pub(crate) struct LigatureGlyphCell {
    pub(crate) font: FontId,
    pub(crate) ch: char,
    code: Option<u8>,
    _metric: Option<CharMetrics>,
    lig_kern_start: Option<u16>,
    leading_auto_kern: Scaled,
    trailing_auto_kern: Scaled,
    pub(crate) provenance: ProvenanceSpan,
    pub(crate) ligature_present: bool,
    pub(crate) left_hit: bool,
    pub(crate) right_hit: bool,
    pub(crate) character_exists: bool,
}

impl LigatureGlyphCell {
    fn new<G>(
        stores: &CommandContext<'_, G>,
        font: FontId,
        ch: char,
        provenance: ProvenanceSpan,
        ligature_present: bool,
        left_hit: bool,
        right_hit: bool,
    ) -> Self {
        let code = font_code(ch).ok();
        let metric = code.and_then(|code| stores.font_char_metrics(font, code));
        let lig_kern_start = code
            .filter(|&code| stores.pdf_font_code(tex_state::PdfFontCode::Tag, font, code) & 1 != 0)
            .and(metric)
            .and_then(|metric| match metric.tag {
                tex_fonts::MetricCharTag::LigKern { start_index, .. } => Some(start_index),
                _ => None,
            });
        let (leading_auto_kern, trailing_auto_kern) = auto_kern_values(stores, font, code);
        Self {
            font,
            ch,
            code,
            _metric: metric,
            lig_kern_start,
            leading_auto_kern,
            trailing_auto_kern,
            provenance,
            ligature_present,
            left_hit,
            right_hit,
            character_exists: stores.font_character_exists(font, ch),
        }
    }

    fn into_pending(self, source: &[LigatureSourceEntry]) -> PendingHRunChar {
        let mut orig = SmallVec::new();
        let mut origins = SmallVec::new();
        for entry in source {
            orig.push(entry.ch);
            origins.push(entry.origin);
        }
        PendingHRunChar {
            font: self.font,
            ch: self.ch,
            orig,
            origins,
            ligature_present: self.ligature_present,
            left_hit: self.left_hit,
            right_hit: self.right_hit,
        }
    }
}

enum LigatureWorkCell {
    Boundary(LigatureBoundaryCell),
    Glyph(LigatureGlyphCell),
    Kern { amount: Scaled, kind: KernKind },
}

struct LigatureWorkNode {
    cell: Option<LigatureWorkCell>,
    previous: Option<usize>,
    next: Option<usize>,
    discard_if_missing: bool,
}

#[derive(Default)]
pub(crate) struct LigatureWorkList {
    nodes: Vec<LigatureWorkNode>,
    provenance: Vec<LigatureSourceEntry>,
    head: Option<usize>,
    tail: Option<usize>,
}

impl LigatureWorkList {
    pub(crate) fn clear(&mut self) {
        self.nodes.clear();
        self.provenance.clear();
        self.head = None;
        self.tail = None;
    }

    fn prepare(&mut self, capacity: usize) {
        self.clear();
        self.nodes
            .reserve(capacity.saturating_sub(self.nodes.capacity()));
        self.provenance
            .reserve(capacity.saturating_sub(self.provenance.capacity()));
    }

    #[cfg(test)]
    fn with_capacity(capacity: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(capacity),
            provenance: Vec::with_capacity(capacity),
            head: None,
            tail: None,
        }
    }

    fn append_source(&mut self, ch: char, origin: OriginId) -> ProvenanceSpan {
        let start = self.provenance.len();
        self.provenance.push(LigatureSourceEntry { ch, origin });
        ProvenanceSpan { start, len: 1 }
    }

    pub(crate) fn source(&self, span: ProvenanceSpan) -> &[LigatureSourceEntry] {
        &self.provenance[span.start..span.end()]
    }

    /// Reports whether the remaining physical main branch has a glyph
    /// boundary at `boundary`, including zero-source kerns at that boundary.
    pub(crate) fn physical_boundary_present(
        &self,
        current: usize,
        current_start: usize,
        current_len: usize,
        boundary: usize,
    ) -> bool {
        let mut position = current_start.saturating_add(current_len);
        if position == boundary {
            return true;
        }
        let mut index = self.nodes.get(current).and_then(|node| node.next);
        while let Some(next) = index {
            index = self.nodes[next].next;
            match self.nodes[next].cell.as_ref() {
                Some(LigatureWorkCell::Glyph(glyph)) => {
                    position = position.saturating_add(glyph.provenance.len);
                    if position == boundary {
                        return true;
                    }
                }
                Some(LigatureWorkCell::Kern { .. }) => {
                    if position == boundary {
                        return true;
                    }
                }
                Some(LigatureWorkCell::Boundary(_)) | None => {}
            }
        }
        false
    }

    /// Counts physical nodes through the first synchronized source boundary.
    /// A kern immediately following the boundary is part of TeX's retained
    /// physical span, matching `nodes_through_character_boundary`.
    pub(crate) fn physical_nodes_through_boundary(
        &self,
        current: usize,
        current_start: usize,
        current_len: usize,
        boundary: usize,
    ) -> usize {
        let mut position = current_start.saturating_add(current_len);
        let mut count = 1usize;
        if position > boundary {
            return count;
        }
        let mut index = self.nodes.get(current).and_then(|node| node.next);
        while let Some(next) = index {
            index = self.nodes[next].next;
            match self.nodes[next].cell.as_ref() {
                Some(LigatureWorkCell::Glyph(glyph)) => {
                    position = position.saturating_add(glyph.provenance.len);
                    count += 1;
                    if position > boundary {
                        break;
                    }
                }
                Some(LigatureWorkCell::Kern { .. }) => {
                    count += 1;
                    if position > boundary {
                        break;
                    }
                }
                Some(LigatureWorkCell::Boundary(_)) | None => {}
            }
        }
        count
    }

    fn merge_source_spans(
        &mut self,
        left: ProvenanceSpan,
        right: ProvenanceSpan,
    ) -> ProvenanceSpan {
        if left.len == 0 {
            return right;
        }
        if right.len == 0 {
            return left;
        }
        if left.end() == right.start {
            return ProvenanceSpan {
                start: left.start,
                len: left.len + right.len,
            };
        }
        // Live glyph cells partition source provenance in logical order, so
        // this is only a defensive path for a future operation that creates
        // non-adjacent spans. It still appends into the reusable tape and
        // never inserts or moves the linked work nodes.
        let start = self.provenance.len();
        self.provenance.extend_from_within(left.start..left.end());
        self.provenance.extend_from_within(right.start..right.end());
        ProvenanceSpan {
            start,
            len: left.len + right.len,
        }
    }

    fn push_back(&mut self, cell: LigatureWorkCell) -> usize {
        let index = self.nodes.len();
        self.nodes.push(LigatureWorkNode {
            cell: Some(cell),
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

    fn insert_between(&mut self, left: usize, right: usize, cell: LigatureWorkCell) -> usize {
        debug_assert_eq!(self.nodes[left].next, Some(right));
        debug_assert_eq!(self.nodes[right].previous, Some(left));
        let inserted = self.append_detached(cell);
        self.nodes[inserted].previous = Some(left);
        self.nodes[inserted].next = Some(right);
        self.nodes[left].next = Some(inserted);
        self.nodes[right].previous = Some(inserted);
        inserted
    }

    fn auto_kern(&self, left: usize, right: usize) -> Option<Scaled> {
        let amount = match (
            self.nodes[left].cell.as_ref(),
            self.nodes[right].cell.as_ref(),
        ) {
            (Some(LigatureWorkCell::Boundary(_)), Some(LigatureWorkCell::Glyph(glyph))) => {
                glyph.leading_auto_kern
            }
            (Some(LigatureWorkCell::Glyph(glyph)), Some(LigatureWorkCell::Boundary(boundary))) => {
                add_scaled(glyph.trailing_auto_kern, boundary.leading_auto_kern)
            }
            (
                Some(LigatureWorkCell::Glyph(left_glyph)),
                Some(LigatureWorkCell::Glyph(right_glyph)),
            ) if left_glyph.font == right_glyph.font => {
                add_scaled(left_glyph.trailing_auto_kern, right_glyph.leading_auto_kern)
            }
            (Some(LigatureWorkCell::Glyph(glyph)), Some(LigatureWorkCell::Glyph(_))) => {
                glyph.trailing_auto_kern
            }
            _ => Scaled::from_raw(0),
        };
        (amount.raw() != 0).then_some(amount)
    }

    fn replace_pair(
        &mut self,
        left: usize,
        right: usize,
        cell: LigatureWorkCell,
        delete_current: bool,
        delete_next: bool,
    ) -> usize {
        let previous = if delete_current {
            self.nodes[left].previous
        } else {
            Some(left)
        };
        let next = if delete_next {
            self.nodes[right].next
        } else {
            Some(right)
        };
        let discard_if_missing = (delete_current && self.nodes[left].discard_if_missing)
            || (delete_next && self.nodes[right].discard_if_missing);
        let replacement = self.append_detached(cell);
        self.nodes[replacement].discard_if_missing = discard_if_missing;
        self.nodes[replacement].previous = previous;
        self.nodes[replacement].next = next;
        if let Some(previous) = previous {
            self.nodes[previous].next = Some(replacement);
        } else {
            self.head = Some(replacement);
        }
        if let Some(next) = next {
            self.nodes[next].previous = Some(replacement);
        } else {
            self.tail = Some(replacement);
        }
        if delete_current {
            self.tombstone(left);
        }
        if delete_next {
            self.tombstone(right);
        }
        replacement
    }

    fn append_detached(&mut self, cell: LigatureWorkCell) -> usize {
        let index = self.nodes.len();
        self.nodes.push(LigatureWorkNode {
            cell: Some(cell),
            previous: None,
            next: None,
            discard_if_missing: false,
        });
        index
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
        self.tombstone(index);
    }

    fn tombstone(&mut self, index: usize) {
        self.nodes[index].cell = None;
        self.nodes[index].previous = None;
        self.nodes[index].next = None;
    }

    fn take(&mut self, index: usize) -> LigatureWorkCell {
        self.nodes[index]
            .cell
            .take()
            .expect("live ligature work cell remains present")
    }
}

struct LigatureWorkReset<'a> {
    work: &'a mut LigatureWorkList,
    restore: Option<(usize, usize, Option<usize>, Option<usize>)>,
}

impl<'a> LigatureWorkReset<'a> {
    fn new(work: &'a mut LigatureWorkList, capacity: usize) -> Self {
        work.prepare(capacity);
        Self {
            work,
            restore: None,
        }
    }

    /// Borrows one nested replay window from the same move-only tape.
    /// Existing parent cells stay in place while the child uses the tail of
    /// both packed vectors; dropping the window restores the parent's links
    /// and truncates only the child suffix.
    fn nested(work: &'a mut LigatureWorkList) -> Self {
        let restore = (
            work.nodes.len(),
            work.provenance.len(),
            work.head,
            work.tail,
        );
        work.head = None;
        work.tail = None;
        Self {
            work,
            restore: Some(restore),
        }
    }
}

impl Drop for LigatureWorkReset<'_> {
    fn drop(&mut self) {
        if let Some((nodes, provenance, head, tail)) = self.restore {
            self.work.nodes.truncate(nodes);
            self.work.provenance.truncate(provenance);
            self.work.head = head;
            self.work.tail = tail;
        } else {
            self.work.clear();
        }
    }
}

/// Runs the TFM ligature machine with caller-owned unresolved work storage.
/// The storage is cleared even when bounded fuel rejects the run, so retaining
/// its capacity cannot retain semantic glyph state across a retry.
#[allow(clippy::too_many_arguments)] // TeX's ligature machine keeps boundary, diagnostics, fuel, and sink explicit.
pub(crate) fn run_tfm_ligature_machine_with_work<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    source: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
    right_boundary: LigatureRightBoundary,
    insert_hyphen_discs: bool,
    fuel: &mut tex_command::CommandFuel,
    work: &mut LigatureWorkList,
    sink: &mut impl FinalHNodeSink,
) -> Result<(), tex_command::CommandError> {
    run_tfm_ligature_machine_with_work_mode(
        stores,
        diagnostic_effects,
        source,
        no_left_boundary,
        right_boundary,
        insert_hyphen_discs,
        fuel,
        work,
        sink,
        false,
    )
}

/// Replays a child branch while a parent reconstitution cursor still owns
/// the tape. The nested window is restored on every exit, including fuel
/// failure, so one warm `HorizontalFlushScratch` remains the sole temporary
/// owner for all branch replays.
#[allow(clippy::too_many_arguments)] // TeX's replay keeps branch, diagnostics, fuel, tape, and sink explicit.
pub(crate) fn run_tfm_ligature_machine_nested_with_work<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    source: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
    right_boundary: LigatureRightBoundary,
    insert_hyphen_discs: bool,
    fuel: &mut tex_command::CommandFuel,
    work: &mut LigatureWorkList,
    sink: &mut impl FinalHNodeSink,
) -> Result<(), tex_command::CommandError> {
    run_tfm_ligature_machine_with_work_mode(
        stores,
        diagnostic_effects,
        source,
        no_left_boundary,
        right_boundary,
        insert_hyphen_discs,
        fuel,
        work,
        sink,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_tfm_ligature_machine_with_work_mode<G>(
    stores: &mut CommandContext<'_, G>,
    diagnostic_effects: &mut DiagnosticEffects,
    source: &[crate::mode::PendingHChar],
    no_left_boundary: bool,
    right_boundary: LigatureRightBoundary,
    insert_hyphen_discs: bool,
    fuel: &mut tex_command::CommandFuel,
    work: &mut LigatureWorkList,
    sink: &mut impl FinalHNodeSink,
    nested: bool,
) -> Result<(), tex_command::CommandError> {
    let work_guard = if nested {
        LigatureWorkReset::nested(work)
    } else {
        LigatureWorkReset::new(work, source.len().saturating_add(4))
    };
    let work = &mut *work_guard.work;
    let Some(first) = source.first() else {
        return Ok(());
    };
    let font = first.font;
    let false_bchar = stores.font_false_boundary_char(font);
    let left_boundary_start = stores.font_lig_kern_start(font, LigKernChar::Boundary);
    if !no_left_boundary {
        work.push_back(LigatureWorkCell::Boundary(LigatureBoundaryCell {
            code: None,
            lig_kern_start: left_boundary_start,
            leading_auto_kern: Scaled::from_raw(0),
        }));
    }
    for entry in source {
        let provenance = work.append_source(entry.ch, entry.origin);
        let glyph = LigatureGlyphCell::new(
            stores, entry.font, entry.ch, provenance, false, false, false,
        );
        work.push_back(LigatureWorkCell::Glyph(glyph));
    }
    match right_boundary {
        LigatureRightBoundary::Suppressed => {}
        LigatureRightBoundary::Font => {
            work.push_back(LigatureWorkCell::Boundary(LigatureBoundaryCell {
                code: None,
                lig_kern_start: left_boundary_start,
                leading_auto_kern: Scaled::from_raw(0),
            }));
        }
        LigatureRightBoundary::Character(ch) => {
            let code = Some(ch);
            work.push_back(LigatureWorkCell::Boundary(LigatureBoundaryCell {
                code,
                lig_kern_start: left_boundary_start,
                leading_auto_kern: auto_kern_values(stores, font, code).0,
            }));
        }
    }

    let mut cursor = work.head;
    while let Some(left_index) = cursor {
        let Some(right_index) = work.nodes[left_index].next else {
            break;
        };
        fuel.charge()?;
        if matches!(
            work.nodes[left_index].cell.as_ref(),
            Some(LigatureWorkCell::Kern { .. })
        ) || matches!(
            work.nodes[right_index].cell.as_ref(),
            Some(LigatureWorkCell::Kern { .. })
        ) {
            cursor = Some(right_index);
            continue;
        }

        let pair = match (
            work.nodes[left_index].cell.as_ref(),
            work.nodes[right_index].cell.as_ref(),
        ) {
            (Some(LigatureWorkCell::Boundary(_)), Some(LigatureWorkCell::Glyph(right))) => right
                .code
                .map(|right| (LigKernChar::Boundary, LigKernChar::Char(right))),
            (Some(LigatureWorkCell::Glyph(left)), Some(LigatureWorkCell::Boundary(right))) => {
                left.code.map(|left| {
                    (
                        LigKernChar::Char(left),
                        right.code.map_or(LigKernChar::Boundary, LigKernChar::Char),
                    )
                })
            }
            (Some(LigatureWorkCell::Glyph(left)), Some(LigatureWorkCell::Glyph(right)))
                if left.font == right.font =>
            {
                left.code
                    .zip(right.code)
                    .map(|(left, right)| (LigKernChar::Char(left), LigKernChar::Char(right)))
            }
            _ => None,
        };
        let false_boundary_match = matches!(
            work.nodes[right_index].cell.as_ref(),
            Some(LigatureWorkCell::Glyph(right))
                if right.font == font && right.code.is_some_and(|code| Some(code) == false_bchar)
        );
        if false_boundary_match {
            work.nodes[right_index].discard_if_missing = true;
        }
        let Some((_left_code, right_code)) = pair else {
            cursor = Some(right_index);
            continue;
        };

        if false_boundary_match {
            if let Some(LigatureWorkCell::Glyph(right)) = work.nodes[right_index].cell.as_ref()
                && !right.character_exists
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

        if let Some(amount) = work.auto_kern(left_index, right_index) {
            let inserted = work.insert_between(
                left_index,
                right_index,
                LigatureWorkCell::Kern {
                    amount,
                    kind: KernKind::Auto,
                },
            );
            cursor = work.nodes[inserted].next;
            continue;
        }

        let left_start = match work.nodes[left_index].cell.as_ref() {
            Some(LigatureWorkCell::Boundary(boundary)) => boundary.lig_kern_start,
            Some(LigatureWorkCell::Glyph(glyph)) => glyph.lig_kern_start,
            Some(LigatureWorkCell::Kern { .. }) | None => None,
        };
        let Some(left_start) = left_start else {
            cursor = Some(right_index);
            continue;
        };
        let Some(command) = stores.font_lig_kern_command_from_start(font, left_start, right_code)
        else {
            cursor = Some(right_index);
            continue;
        };
        match command {
            LigKernCommand::Kern(amount) => {
                let inserted = work.insert_between(
                    left_index,
                    right_index,
                    LigatureWorkCell::Kern {
                        amount,
                        kind: KernKind::Font,
                    },
                );
                cursor = work.nodes[inserted].next;
            }
            LigKernCommand::Ligature(lig) => {
                let (left_span, left_hit, left_is_boundary) =
                    match work.nodes[left_index].cell.as_ref() {
                        Some(LigatureWorkCell::Glyph(glyph)) => {
                            (glyph.provenance, glyph.left_hit, false)
                        }
                        Some(LigatureWorkCell::Boundary(_)) => (ProvenanceSpan::EMPTY, false, true),
                        Some(LigatureWorkCell::Kern { .. }) | None => unreachable!(),
                    };
                let (right_span, right_hit, right_is_boundary) =
                    match work.nodes[right_index].cell.as_ref() {
                        Some(LigatureWorkCell::Glyph(glyph)) => {
                            (glyph.provenance, glyph.right_hit, false)
                        }
                        Some(LigatureWorkCell::Boundary(_)) => (ProvenanceSpan::EMPTY, false, true),
                        Some(LigatureWorkCell::Kern { .. }) | None => unreachable!(),
                    };
                let provenance = work.merge_source_spans(
                    if lig.delete_current {
                        left_span
                    } else {
                        ProvenanceSpan::EMPTY
                    },
                    if lig.delete_next {
                        right_span
                    } else {
                        ProvenanceSpan::EMPTY
                    },
                );
                let replacement = LigatureGlyphCell::new(
                    stores,
                    font,
                    char::from(lig.replacement),
                    provenance,
                    true,
                    left_is_boundary || left_hit,
                    right_is_boundary || right_hit,
                );
                let replacement = work.replace_pair(
                    left_index,
                    right_index,
                    LigatureWorkCell::Glyph(replacement),
                    lig.delete_current,
                    lig.delete_next,
                );
                let op_byte = lig.pass_over * 4
                    + u8::from(!lig.delete_current) * 2
                    + u8::from(!lig.delete_next);
                cursor = Some(if lig.delete_current {
                    replacement
                } else {
                    left_index
                });
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

    // A literal hyphen discretionary belongs to the output list, but its
    // position is known only after any following auto kerns. Carry that
    // pending decision as a scalar until the position is known instead of
    // moving a full `Node` through temporary owners.
    let mut pending_literal_hyphen_disc = false;
    let mut index = work.head;
    while let Some(current) = index {
        index = work.nodes[current].next;
        let is_auto_kern = matches!(
            work.nodes[current].cell.as_ref(),
            Some(LigatureWorkCell::Kern {
                kind: KernKind::Auto,
                ..
            })
        );
        if !is_auto_kern && pending_literal_hyphen_disc {
            sink.explicit_hyphen_disc(stores);
            pending_literal_hyphen_disc = false;
        }
        let discard_if_missing = work.nodes[current].discard_if_missing;
        match work.take(current) {
            LigatureWorkCell::Boundary(_) => {}
            LigatureWorkCell::Glyph(glyph) => {
                if discard_if_missing && !glyph.character_exists {
                    crate::diagnostics::report_missing_character_warning(
                        stores,
                        diagnostic_effects,
                        glyph.font,
                        glyph.ch,
                        false,
                    );
                    continue;
                }
                let source_last = work.source(glyph.provenance).last().map(|entry| entry.ch);
                pending_literal_hyphen_disc = literal_hyphen_disc_enabled_source(
                    stores,
                    &glyph,
                    source_last,
                    insert_hyphen_discs,
                );
                sink.glyph_cell_with_work(stores, current, glyph, work, diagnostic_effects, fuel)?;
            }
            LigatureWorkCell::Kern { amount, kind } => sink.kern_with_work(
                stores,
                current,
                amount,
                kind,
                work,
                diagnostic_effects,
                fuel,
            )?,
        }
    }
    if pending_literal_hyphen_disc {
        sink.explicit_hyphen_disc(stores);
    }
    Ok(())
}

fn auto_kern_values<G>(
    stores: &CommandContext<'_, G>,
    font: FontId,
    code: Option<u8>,
) -> (Scaled, Scaled) {
    let configuration = stores.pdf_font_configuration();
    let leading = if configuration.prepends_kerns() {
        code.map_or(Scaled::from_raw(0), |code| {
            scaled_font_code(
                stores,
                font,
                stores.pdf_font_code(tex_state::PdfFontCode::Knbc, font, code),
            )
        })
    } else {
        Scaled::from_raw(0)
    };
    let trailing = if configuration.appends_kerns() {
        code.map_or(Scaled::from_raw(0), |code| {
            scaled_font_code(
                stores,
                font,
                stores.pdf_font_code(tex_state::PdfFontCode::Knac, font, code),
            )
        })
    } else {
        Scaled::from_raw(0)
    };
    (leading, trailing)
}

fn literal_hyphen_disc_enabled_source<G>(
    stores: &mut CommandContext<'_, G>,
    current: &LigatureGlyphCell,
    source_last: Option<char>,
    enabled: bool,
) -> bool {
    enabled && stores.font_hyphen_char(current.font) == source_last.unwrap_or(current.ch) as i32
}

pub(crate) fn add_scaled(left: Scaled, right: Scaled) -> Scaled {
    left.checked_add(right)
        .expect("pdfTeX inter-character kern adjustment fits Scaled")
}

pub(crate) fn adjust_interword_glue<G>(
    stores: &CommandContext<'_, G>,
    nodes: tex_state::node_arena::NodeCursor<'_>,
    spec: &mut GlueSpec,
) {
    let mut glyph = None;
    for node in nodes.iter().rev() {
        match node {
            tex_state::NodeView::Char { font, ch, .. }
            | tex_state::NodeView::Lig { font, ch, .. } => {
                glyph = u8::try_from(ch as u32).ok().map(|code| (font, code));
                break;
            }
            tex_state::NodeView::Kern {
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
    let Some((font, ch)) = last_font_char(nest.current_list().nodes(stores)) else {
        return Ok(());
    };
    let Ok(code) = font_code(ch) else {
        return Ok(());
    };
    let Some(metrics) = stores.font_char_metrics(font, code) else {
        return Ok(());
    };
    nest.current_list_mutation().push(
        stores,
        Node::Kern {
            amount: metrics.italic_correction,
            kind: KernKind::Explicit,
        },
    );
    Ok(())
}

pub(crate) fn last_font_char(
    nodes: tex_state::node_arena::NodeCursor<'_>,
) -> Option<(tex_state::ids::FontId, char)> {
    match nodes.last()? {
        tex_state::node_arena::NodeView::Char { font, ch, .. }
        | tex_state::node_arena::NodeView::Lig { font, ch, .. } => Some((font, ch)),
        _ => None,
    }
}

pub(crate) fn font_code(ch: char) -> Result<u8, ()> {
    u8::try_from(ch as u32).map_err(|_| ())
}

#[cfg(test)]
mod tests;
