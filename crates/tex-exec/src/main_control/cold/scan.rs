//! Operand collection for uncommon and structurally large commands.
//!
//! Every scanner borrows the session's canonical command processor and
//! returns one [`ColdOperation`]; it never creates another input machine.

use super::super::*;
use super::operation::*;
use super::support::*;

#[allow(clippy::too_many_arguments)] // mirrors the typed main-control context
pub(in crate::main_control) fn scan(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    global: bool,
    mode: Mode,
    boxes: &ReplayBoxes,
    innermost_group: Option<GroupKind>,
    job_is_all_over: bool,
    display_eq_no: bool,
    set_box_allowed: bool,
    shown_mode: &mut Option<Mode>,
) -> Result<ColdOperation, ExecError> {
    match command.meaning() {
        Meaning::CharToken {
            cat: Catcode::BeginGroup,
            ..
        } => unreachable!("ordinary group entry is owned by fused hot dispatch"),
        // TeX82 §1068's `handle_right_brace` sends three of its `cur_group`
        // cases to §1069's `extra_right_brace`, which names the group opener
        // the brace was mistaken for; every other unmatched brace is §1068's
        // own `bottom_level` arm.
        Meaning::CharToken {
            cat: Catcode::EndGroup,
            ..
        } => Ok(ColdOperation::ExtraRightBrace {
            forgotten: match innermost_group {
                Some(GroupKind::SemiSimple) => Some(ForgottenGroupOpener::EndGroup),
                Some(GroupKind::MathShift) => Some(ForgottenGroupOpener::MathShift),
                Some(GroupKind::MathLeft) => Some(ForgottenGroupOpener::Right),
                _ => None,
            },
        }),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::BeginGroup) => {
            unreachable!("semi-simple group entry is owned by fused hot dispatch")
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndGroup)
            if innermost_group == Some(GroupKind::SemiSimple) =>
        {
            unreachable!("semi-simple group exit is owned by fused hot dispatch")
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::EndGroup) => {
            // TeX82 §1215's `\let` copies `cur_cmd`/`cur_chr`, so every
            // user control sequence with `end_group` meaning takes §1063's
            // dispatch irrespective of its spelling. Inaccessible alignment
            // sentinels have the distinct `EndV`/`EndTemplate` meanings and
            // remain owned by their dedicated paths.
            scan_off_save(processor, command, innermost_group)
        }
        // TeX82 §1094's `hmode+stop,...: head_for_vmode`. §1095's
        // unrestricted branch (`mode>0`) backs the stop up, then backs an
        // inserted `\\par` up ahead of it, so the stop is retried in the
        // enclosing vertical mode. The command core owns both backups;
        // replay merely processes the resulting `\\par`.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::End | UnexpandablePrimitive::Dump,
        ) if mode == Mode::Horizontal => {
            processor
                .recover_stop_for_vertical_mode(command)
                .map_err(command_error)?;
            Ok(ColdOperation::Continue)
        }
        // §1095's restricted-`hmode` branch (`mode<0`, e.g. inside an
        // `\\hbox`): `if cur_cmd<>hrule then off_save`. `\\par` has no
        // meaning there, so §1064's fully general recovery closes the
        // enclosing group instead, exactly as the `\\vskip` family above.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::End | UnexpandablePrimitive::Dump,
        ) if mode == Mode::RestrictedHorizontal => {
            scan_off_save(processor, command, innermost_group)
        }
        // §1046's "math-only cases in non-math modes, or vice versa" table
        // lists `mmode+stop`, so §1047's `insert_dollar_sign` closes the math
        // first and retries the stop in the resulting mode.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::End | UnexpandablePrimitive::Dump,
        ) if matches!(mode, Mode::Math | Mode::DisplayMath) => {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ColdOperation::MissingMathShift)
        }
        // §1045's `vmode+stop: if its_all_over then return` -- "this is the
        // only way out" of `main_control`. §1054's `its_all_over` is the one
        // general mechanism: the job ends only when the current page and the
        // contribution list are both empty and the last output was not a dead
        // cycle. Otherwise the stop is backed up and residual material is
        // ejected by appending `\\hbox to \\hsize{}`, `\\vfill`, and
        // `\\penalty-'10000000000` and calling §994's `build_page`; whether
        // that reaches `\\output` at all, and with what `\\box255`, is
        // §1005/§1012's decision, never this dispatch's.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::End | UnexpandablePrimitive::Dump,
        ) => {
            // §1051's `privileged`: `mode>0` only. Internal vertical mode
            // (inside a `\\vbox`, an `\\insert`, or `\\output` itself) reports
            // an illegal case and leaves the job running.
            if mode != Mode::Vertical {
                return Ok(ColdOperation::IllegalStop {
                    token: command.spelling().semantic_token(),
                });
            }
            if job_is_all_over {
                // §1335's `final_cleanup` unwinds the input stack that
                // `main_control`'s return has abandoned.
                let incomplete_conditions = processor.final_cleanup();
                return Ok(ColdOperation::End {
                    dump: matches!(
                        command.meaning(),
                        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dump)
                    ),
                    incomplete_conditions,
                });
            }
            processor.back_input(command).map_err(command_error)?;
            Ok(ColdOperation::EjectResidualPage)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count) => {
            let index = processor
                .scan_profile_register_index()
                .map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ColdOperation::Count {
                index,
                value,
                global,
            })
        }
        Meaning::CountRegister(index) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ColdOperation::Count {
                index,
                value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
            let index = processor
                .scan_profile_register_index()
                .map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ColdOperation::Dimen {
                index,
                value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Wd
            | UnexpandablePrimitive::Ht
            | UnexpandablePrimitive::Dp),
        ) => {
            let index = processor
                .scan_profile_register_index()
                .map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            let dimension = match primitive {
                UnexpandablePrimitive::Wd => tex_state::BoxDimension::Width,
                UnexpandablePrimitive::Ht => tex_state::BoxDimension::Height,
                UnexpandablePrimitive::Dp => tex_state::BoxDimension::Depth,
                _ => unreachable!(),
            };
            Ok(ColdOperation::BoxDimensionAssignment {
                index,
                dimension,
                value,
                global,
            })
        }
        Meaning::DimenRegister(index) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ColdOperation::Dimen {
                index,
                value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => {
            let index = processor
                .scan_profile_register_index()
                .map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_glue(false).map_err(command_error)?.value;
            let source_identity = processor.scanned_glue_identity();
            let source_skip_index = processor.scanned_glue_skip_index();
            Ok(ColdOperation::Skip {
                index,
                value,
                source_identity,
                source_skip_index,
                redundant: false,
                reassigning: false,
                global,
            })
        }
        Meaning::SkipRegister(index) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_glue(false).map_err(command_error)?.value;
            let source_identity = processor.scanned_glue_identity();
            let source_skip_index = processor.scanned_glue_skip_index();
            Ok(ColdOperation::Skip {
                index,
                value,
                source_identity,
                source_skip_index,
                redundant: false,
                reassigning: false,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
            let index = processor
                .scan_profile_register_index()
                .map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_glue(true).map_err(command_error)?.value;
            let source_identity = processor.scanned_glue_identity();
            Ok(ColdOperation::Muskip {
                index,
                value,
                source_identity,
                redundant: false,
                reassigning: false,
                global,
            })
        }
        Meaning::MuskipRegister(index) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_glue(true).map_err(command_error)?.value;
            let source_identity = processor.scanned_glue_identity();
            Ok(ColdOperation::Muskip {
                index,
                value,
                source_identity,
                redundant: false,
                reassigning: false,
                global,
            })
        }
        // TeX82 §458 leaves `scan_glue` entirely in the command machine.
        // Main control receives only its completed typed specification, so a
        // u-template's numeric operand retains the canonical `back_input`
        // and replay sequence before this layer appends the glue node.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HSkip) => {
            let value = processor.scan_glue(false).map_err(command_error)?.value;
            Ok(ColdOperation::HorizontalSkip { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Kern) => {
            let amount = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ColdOperation::Kern { amount })
        }
        // TeX82 §1102's `any_mode(break_penalty): append_penalty` (§1103:
        // `scan_int; tail_append(new_penalty(cur_val))`). `\penalty` never
        // switches mode -- it appends directly to whatever list (main
        // vertical, horizontal, restricted horizontal, or math) is current.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Penalty) => {
            let amount = processor.scan_integer().map_err(command_error)?.value;
            Ok(ColdOperation::Penalty { amount })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ControlSpace) => {
            Ok(ColdOperation::ControlSpace)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevDepth) => {
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                let _ = processor.scan_optional_equals().map_err(command_error)?;
                let value = processor.scan_dimension().map_err(command_error)?.value;
                Ok(ColdOperation::PrevDepth { value })
            } else {
                Ok(ColdOperation::IllegalPrevDepth {
                    token: command.spelling().semantic_token(),
                })
            }
        }
        // TeX82 §1265's `any_mode(set_interaction): prefixed_command` ->
        // `new_interaction` (§1264): `interaction:=cur_chr`. The four
        // primitives differ only in the fixed `chr_code` each was installed
        // with (§1264's four `primitive("...",set_interaction,...)` calls),
        // so there is no operand scan of any kind -- the target level is
        // selected purely from which primitive was delivered, exactly like
        // `\unpenalty`/`\unkern`/`\unskip` above. `interaction` is a plain
        // global Pascal variable outside `eqtb`, so this assignment is never
        // grouped/undone and ignores `\global`/`\globaldefs` entirely, unlike
        // ordinary parameter assignments.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::BatchMode
            | UnexpandablePrimitive::NonstopMode
            | UnexpandablePrimitive::ScrollMode
            | UnexpandablePrimitive::ErrorStopMode),
        ) => Ok(ColdOperation::SetInteractionMode(primitive)),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::InteractionMode) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ColdOperation::SetInteractionModeValue {
                value,
                context: processor.error_context(),
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SpaceFactor) => {
            if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) {
                let _ = processor.scan_optional_equals().map_err(command_error)?;
                let value = processor.scan_integer().map_err(command_error)?.value;
                Ok(ColdOperation::SpaceFactor { value })
            } else {
                Ok(ColdOperation::IllegalSpaceFactor {
                    token: command.spelling().semantic_token(),
                })
            }
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PrevGraf) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ColdOperation::PrevGraf { value })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
            let origin = material_origin(processor, &command);
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ColdOperation::CharacterCode {
                value,
                origin,
                suppress_left_boundary: false,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Accent) => Ok(ColdOperation::Accent(
            processor.scan_accent().map_err(command_error)?,
        )),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Discretionary) => {
            Ok(ColdOperation::DiscretionaryOpening(
                processor
                    .scan_discretionary_opening()
                    .map_err(command_error)?,
            ))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::DiscretionaryHyphen) => {
            Ok(ColdOperation::DiscretionaryHyphen {
                origin: material_origin(processor, &command),
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::HFil
            | UnexpandablePrimitive::HFill
            | UnexpandablePrimitive::HSs
            | UnexpandablePrimitive::HFilNeg),
        ) => Ok(ColdOperation::FixedHorizontalGlue { primitive }),
        // `\vskip`/`\vfil`/`\vfill`/`\vss`/`\vfilneg` are legal only in
        // vertical mode. TeX82 §1046's "math-only cases in non-math modes, or
        // vice versa" table lists `mmode+vskip` (and the fil variants) among
        // the cases §1047's `insert_dollar_sign` recovers from, identically
        // to `mmode+hrule` above.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::VSkip
            | UnexpandablePrimitive::VFil
            | UnexpandablePrimitive::VFill
            | UnexpandablePrimitive::VSs
            | UnexpandablePrimitive::VFilNeg,
        ) if matches!(mode, Mode::Math | Mode::DisplayMath) => {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ColdOperation::MissingMathShift)
        }
        // §1095's `head_for_vmode` distinguishes unrestricted `hmode`
        // (`mode>=0`) from restricted `hmode` (`mode<0`, e.g. inside an
        // `\hbox`): only the unrestricted case takes the simple
        // "back up, insert `\par`, retry" path that
        // `recover_stop_for_vertical_mode` implements.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::VSkip
            | UnexpandablePrimitive::VFil
            | UnexpandablePrimitive::VFill
            | UnexpandablePrimitive::VSs
            | UnexpandablePrimitive::VFilNeg,
        ) if mode == Mode::Horizontal => {
            processor
                .recover_stop_for_vertical_mode(command)
                .map_err(command_error)?;
            Ok(ColdOperation::Continue)
        }
        // §1095's `head_for_vmode`'s restricted-`hmode` branch
        // (`mode<0`): `if cur_cmd<>hrule then off_save`. Unlike the
        // unrestricted case above, restricted horizontal mode (e.g. inside
        // an `\hbox`) cannot simply insert `\par` and retry -- `\par` has no
        // meaning there -- so TeX instead runs the fully general §1064
        // `off_save` recovery against whatever group the `\hbox` (or other
        // box-making construct) opened.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::VSkip
            | UnexpandablePrimitive::VFil
            | UnexpandablePrimitive::VFill
            | UnexpandablePrimitive::VSs
            | UnexpandablePrimitive::VFilNeg,
        ) if mode == Mode::RestrictedHorizontal => {
            scan_off_save(processor, command, innermost_group)
        }
        // TeX82 §1057's `vmode+vskip: append_glue` (using `abs(mode)`, so both
        // outer `Vertical` and `InternalVertical` match `vmode`).
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSkip)
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) =>
        {
            let value = processor.scan_glue(false).map_err(command_error)?.value;
            Ok(ColdOperation::VerticalSkip { value })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::VFil
            | UnexpandablePrimitive::VFill
            | UnexpandablePrimitive::VSs
            | UnexpandablePrimitive::VFilNeg),
        ) if matches!(mode, Mode::Vertical | Mode::InternalVertical) => {
            Ok(ColdOperation::FixedVerticalGlue { primitive })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Indent) => {
            Ok(ColdOperation::ParagraphIndent { indent: true })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoIndent) => {
            Ok(ColdOperation::ParagraphIndent { indent: false })
        }
        // pdftex.web §1092 installs `quitvmode` as an ordinary `start_par`
        // command: vertical modes begin an indented paragraph, while
        // horizontal (including restricted hmode) and math modes do nothing.
        // Unlike `\indent`, the nonvertical cases must not append an indent
        // box or noad.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::QuitVMode) => {
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                Ok(ColdOperation::ParagraphStart)
            } else {
                Ok(ColdOperation::Continue)
            }
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ParShape) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let count = processor
                .scan_integer()
                .map_err(command_error)?
                .value
                .max(0) as usize;
            let mut lines = Vec::with_capacity(count);
            for _ in 0..count {
                lines.push(ParagraphShapeLine {
                    indent: processor.scan_dimension().map_err(command_error)?.value,
                    width: processor.scan_dimension().map_err(command_error)?.value,
                });
            }
            Ok(ColdOperation::ParagraphShape { lines, global })
        }
        // e-TeX 2.6 change [49.1248] extends TeX82 §1248's `set_shape`:
        // after the optional equals and integer count, the four penalty-array
        // selectors scan exactly `max(count, 0)` integer values. Keeping the
        // complete scan in this typed step preserves retry atomicity.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::InterLinePenalties
            | UnexpandablePrimitive::ClubPenalties
            | UnexpandablePrimitive::WidowPenalties
            | UnexpandablePrimitive::DisplayWidowPenalties),
        ) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let count = processor
                .scan_integer()
                .map_err(command_error)?
                .value
                .max(0) as usize;
            let mut values = Vec::new();
            values
                .try_reserve_exact(count)
                .map_err(|_| ExecError::ArithmeticOverflow)?;
            for _ in 0..count {
                values.push(processor.scan_integer().map_err(command_error)?.value);
            }
            let kind = match primitive {
                UnexpandablePrimitive::InterLinePenalties => PenaltyArrayKind::InterLine,
                UnexpandablePrimitive::ClubPenalties => PenaltyArrayKind::Club,
                UnexpandablePrimitive::WidowPenalties => PenaltyArrayKind::Widow,
                UnexpandablePrimitive::DisplayWidowPenalties => PenaltyArrayKind::DisplayWidow,
                _ => unreachable!("outer match restricts primitive to e-TeX penalty arrays"),
            };
            Ok(ColdOperation::PenaltyArray {
                kind,
                values,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Toks) => {
            let owner = command.control_sequence().ok_or(ExecError::MissingToken {
                context: "token-list assignment owner",
            })?;
            let assignment = processor
                .scan_token_register_assignment(owner)
                .map_err(command_error)?;
            Ok(ColdOperation::Toks {
                index: assignment.index,
                tokens: assignment.tokens,
                global,
            })
        }
        Meaning::ToksRegister(index) => {
            let owner = command.control_sequence().ok_or(ExecError::MissingToken {
                context: "token-list assignment owner",
            })?;
            Ok(ColdOperation::Toks {
                index,
                tokens: processor
                    .scan_token_register_value(owner)
                    .map_err(command_error)?,
                global,
            })
        }
        Meaning::IntParam(index) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ColdOperation::IntParam {
                index,
                value,
                global,
            })
        }
        Meaning::DimenParam(index) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ColdOperation::DimenParam {
                index,
                value,
                global,
            })
        }
        // TeX82 §1210 lists `set_page_dimen` and `set_page_int` among
        // `prefixed_command`'s ordinary assignment forms, and §1242 routes
        // them to `alter_page_so_far` (§1245) and `alter_integer` (§1246).
        // Both scan exactly like the `\dimen`/`\count` parameter arms above,
        // and both deliberately drop `global`: §1242's own comment ("these
        // definitions are always global") applies because `page_so_far`,
        // `dead_cycles`, and `insert_penalties` are engine variables rather
        // than `eqtb` entries, so neither `\global` nor `\globaldefs` has
        // anything to scope.
        Meaning::PageDimension(dimension) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ColdOperation::PageDimension { dimension, value })
        }
        Meaning::PageInteger(integer) => {
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ColdOperation::PageInteger { integer, value })
        }
        Meaning::TokParam(index) => {
            let owner = command.control_sequence().ok_or(ExecError::MissingToken {
                context: "token-list assignment owner",
            })?;
            let tokens = processor
                .scan_token_parameter_assignment(TokParam::new(index), owner)
                .map_err(command_error)?;
            Ok(ColdOperation::TokParam {
                index,
                tokens: tokens.tokens,
                global,
            })
        }
        Meaning::GlueParam(index) => {
            let assignment = processor
                .scan_glue_parameter_assignment(index, false)
                .map_err(command_error)?;
            Ok(ColdOperation::GlueParam {
                index: assignment.index,
                value: assignment.value,
                global,
            })
        }
        Meaning::MuGlueParam(index) => {
            let assignment = processor
                .scan_glue_parameter_assignment(index, true)
                .map_err(command_error)?;
            Ok(ColdOperation::GlueParam {
                index: assignment.index,
                value: assignment.value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::OpenIn
            | UnexpandablePrimitive::CloseIn
            | UnexpandablePrimitive::Read
            | UnexpandablePrimitive::ReadLine),
        ) => {
            // §1214 fixes the effective scope before §1225 calls
            // `read_toks`; carry that scope across the typed apply seam.
            Ok(ColdOperation::InputStream {
                request: processor
                    .scan_input_stream_request(primitive, global)
                    .map_err(command_error)?,
                resource: None,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Font) => {
            // TeX82 §1257's `define(u,set_font,null_font)` precedes the file
            // name scan, so like §1224's provisional `\relax` it takes the
            // scope the eventual definition would take, `\globaldefs`
            // included.
            let request = processor
                .scan_font_definition(global)
                .map_err(command_error)?;
            Ok(ColdOperation::FontDefinition {
                request,
                resource: Box::new(None),
                global,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfCopyFont
            | UnexpandablePrimitive::LetterspaceFont),
        ) => {
            let kind = match primitive {
                UnexpandablePrimitive::PdfCopyFont => GeneratedFontKind::Copy,
                UnexpandablePrimitive::LetterspaceFont => GeneratedFontKind::Letterspace,
                _ => unreachable!(),
            };
            Ok(ColdOperation::GeneratedFontDefinition {
                definition: processor
                    .scan_generated_font_definition(kind, global)
                    .map_err(command_error)?,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfXImage | UnexpandablePrimitive::PdfRefXImage),
        ) => {
            // pdftex.web §§1551–1552 begin both image cases with
            // `check_pdfoutput`, before version checking, image-object
            // allocation, every rule/attr/named/page/colorspace/page-box/file
            // scan, host image lookup, reference validation, whatsit
            // allocation, or list mutation.
            if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
                let name = match primitive {
                    UnexpandablePrimitive::PdfXImage => "pdfximage",
                    UnexpandablePrimitive::PdfRefXImage => "pdfrefximage",
                    _ => unreachable!(),
                };
                return Err(ExecError::PdfExtensionInDviMode(name));
            }
            if primitive == UnexpandablePrimitive::PdfRefXImage {
                return Ok(ColdOperation::PdfRefXImage {
                    object: processor.scan_integer().map_err(command_error)?.value,
                });
            }
            Ok(ColdOperation::PdfXImage {
                request: processor.scan_pdf_image_request().map_err(command_error)?,
                // This placeholder is replaced after the processor borrow;
                // it can never reach application.
                resource: PdfImageResource::Unavailable,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSetRandomSeed) => {
            let seed = processor.scan_integer().map_err(command_error)?.value;
            Ok(ColdOperation::PdfSetRandomSeed {
                seed: seed.saturating_abs(),
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfResetTimer) => {
            Ok(ColdOperation::PdfResetTimer)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfInterwordSpaceOn) => {
            Ok(ColdOperation::PdfInterwordSpace(
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOn,
            ))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfInterwordSpaceOff) => {
            Ok(ColdOperation::PdfInterwordSpace(
                tex_state::node::PdfAccessibilityControl::InterwordSpaceOff,
            ))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfFakeSpace) => Ok(
            ColdOperation::PdfInterwordSpace(tex_state::node::PdfAccessibilityControl::FakeSpace),
        ),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRunningLinkOn) => {
            Ok(ColdOperation::PdfRunningLink(true))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfRunningLinkOff) => {
            Ok(ColdOperation::PdfRunningLink(false))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfSpaceFont) => {
            Ok(ColdOperation::PdfSpaceFont(
                processor
                    .scan_balanced_text(true)
                    .map_err(command_error)?
                    .tokens,
            ))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfObject) => {
            // pdftex.web §§1535 and 1542 call `check_pdfoutput` before
            // `reserveobjnum`, `useobjnum`, the integer, stream/attr/file
            // options, body scan, or allocation.
            if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfobj"));
            }
            Ok(ColdOperation::PdfObject(
                processor.scan_pdf_object_request().map_err(command_error)?,
            ))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfReferenceObject) => {
            // pdftex.web §1544 calls `check_pdfoutput` before `scan_int`,
            // object validation, whatsit allocation, or list mutation.
            if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
                return Err(ExecError::PdfExtensionInDviMode("pdfrefobj"));
            }
            Ok(ColdOperation::PdfReferenceObject(
                processor
                    .scan_pdf_reference_object_request()
                    .map_err(command_error)?,
            ))
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfXForm | UnexpandablePrimitive::PdfRefXForm),
        ) => {
            // pdftex.web §§1548–1549 call `check_pdfoutput` before form-object
            // allocation, either option scan, the box-register/integer scan,
            // object validation, whatsit allocation, or list mutation.
            if processor.int_param(IntParam::PDF_OUTPUT) <= 0 {
                let name = match primitive {
                    UnexpandablePrimitive::PdfXForm => "pdfxform",
                    UnexpandablePrimitive::PdfRefXForm => "pdfrefxform",
                    _ => unreachable!(),
                };
                return Err(ExecError::PdfExtensionInDviMode(name));
            }
            Ok(ColdOperation::PdfForm(
                processor
                    .scan_pdf_form_request(primitive)
                    .map_err(command_error)?,
            ))
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfInfo
            | UnexpandablePrimitive::PdfCatalog
            | UnexpandablePrimitive::PdfNames
            | UnexpandablePrimitive::PdfTrailer
            | UnexpandablePrimitive::PdfTrailerId),
        ) => Ok(ColdOperation::PdfDocumentFragment(
            processor
                .scan_pdf_document_fragment_request(primitive)
                .map_err(command_error)?,
        )),
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfLiteral
            | UnexpandablePrimitive::PdfSetMatrix
            | UnexpandablePrimitive::PdfSave
            | UnexpandablePrimitive::PdfRestore
            | UnexpandablePrimitive::PdfColorStack
            | UnexpandablePrimitive::PdfSavePos
            | UnexpandablePrimitive::PdfSnapRefPoint
            | UnexpandablePrimitive::PdfSnapY
            | UnexpandablePrimitive::PdfSnapYComp),
        ) => {
            // pdftex.web §§1524 and 1563 run `check_pdfoutput` before each
            // extension's operand scanner. `\pdfsavepos` is the sole member
            // of this graphics family that remains available in DVI mode.
            if primitive != UnexpandablePrimitive::PdfSavePos
                && processor.int_param(IntParam::PDF_OUTPUT) <= 0
            {
                let name = match primitive {
                    UnexpandablePrimitive::PdfLiteral => "pdfliteral",
                    UnexpandablePrimitive::PdfSetMatrix => "pdfsetmatrix",
                    UnexpandablePrimitive::PdfSave => "pdfsave",
                    UnexpandablePrimitive::PdfRestore => "pdfrestore",
                    UnexpandablePrimitive::PdfColorStack => "pdfcolorstack",
                    UnexpandablePrimitive::PdfSnapRefPoint => "pdfsnaprefpoint",
                    UnexpandablePrimitive::PdfSnapY => "pdfsnapy",
                    UnexpandablePrimitive::PdfSnapYComp => "pdfsnapycomp",
                    UnexpandablePrimitive::PdfSavePos => unreachable!(),
                    _ => unreachable!(),
                };
                return Err(ExecError::PdfExtensionInDviMode(name));
            }
            Ok(ColdOperation::PdfGraphics(
                processor
                    .scan_pdf_graphics_request(primitive)
                    .map_err(command_error)?
                    .expect("graphics primitive has a typed request"),
            ))
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfAnnot
            | UnexpandablePrimitive::PdfStartLink
            | UnexpandablePrimitive::PdfEndLink
            | UnexpandablePrimitive::PdfOutline
            | UnexpandablePrimitive::PdfDest
            | UnexpandablePrimitive::PdfThread
            | UnexpandablePrimitive::PdfStartThread
            | UnexpandablePrimitive::PdfEndThread),
        ) => {
            if matches!(
                primitive,
                UnexpandablePrimitive::PdfAnnot
                    | UnexpandablePrimitive::PdfStartLink
                    | UnexpandablePrimitive::PdfEndLink
                    | UnexpandablePrimitive::PdfOutline
                    | UnexpandablePrimitive::PdfDest
                    | UnexpandablePrimitive::PdfThread
                    | UnexpandablePrimitive::PdfStartThread
                    | UnexpandablePrimitive::PdfEndThread
            ) && processor.int_param(IntParam::PDF_OUTPUT) <= 0
            {
                let name = match primitive {
                    UnexpandablePrimitive::PdfAnnot => "pdfannot",
                    UnexpandablePrimitive::PdfStartLink => "pdfstartlink",
                    UnexpandablePrimitive::PdfEndLink => "pdfendlink",
                    UnexpandablePrimitive::PdfOutline => "pdfoutline",
                    UnexpandablePrimitive::PdfDest => "pdfdest",
                    UnexpandablePrimitive::PdfThread => "pdfthread",
                    UnexpandablePrimitive::PdfStartThread => "pdfstartthread",
                    UnexpandablePrimitive::PdfEndThread => "pdfendthread",
                    _ => unreachable!(),
                };
                return Err(ExecError::PdfExtensionInDviMode(name));
            }
            // pdftex.web §1561 rejects both link boundary commands in
            // vertical mode immediately after `check_pdfoutput`, before
            // `\pdfstartlink` scans its rule, attributes, or action.  Keep
            // this restriction on the scanning side of the typed-request
            // boundary so malformed operands cannot mask the mode error.
            if matches!(
                primitive,
                UnexpandablePrimitive::PdfStartLink | UnexpandablePrimitive::PdfEndLink
            ) && matches!(mode, Mode::Vertical | Mode::InternalVertical)
            {
                return Err(ExecError::PdfLinkInVerticalMode(match primitive {
                    UnexpandablePrimitive::PdfStartLink => "pdfstartlink",
                    UnexpandablePrimitive::PdfEndLink => "pdfendlink",
                    _ => unreachable!(),
                }));
            }
            Ok(ColdOperation::PdfNavigation(
                processor
                    .scan_pdf_navigation_request(primitive)
                    .map_err(command_error)?,
            ))
        }
        Meaning::Font(font) => Ok(ColdOperation::FontSelect {
            font,
            selector: command.control_sequence(),
            global,
        }),
        // tex.web §578's `find_font_dimen` scans the number *and* the font
        // identifier before it decides the number is unusable, and §1253 then
        // scans `=<dimen>` either way; the whole assignment is consumed even
        // when §579 rejects it.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::FontDimen) => {
            let number = processor.scan_integer().map_err(command_error)?.value;
            let font = processor.scan_font_selector().map_err(command_error)?;
            // §579 reports from inside `find_font_dimen`, so its `show_context`
            // splits here -- after the font identifier and before `=<dimen>`.
            let recovery_context =
                (!processor.font_dimen_writable(font, number)).then(|| processor.error_context());
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_dimension().map_err(command_error)?.value;
            Ok(ColdOperation::FontDimen {
                font,
                number,
                value,
                recovery_context,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::HyphenChar | UnexpandablePrimitive::SkewChar),
        ) => {
            let font = processor.scan_font_selector().map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ColdOperation::FontInteger {
                font,
                skew: primitive == UnexpandablePrimitive::SkewChar,
                value,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::OpenOut) => {
            let stream = processor
                .scan_restricted_integer(RestrictedIntegerClass::FourBit)
                .map_err(command_error)?
                .value as u8;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let file_name = processor.scan_file_name().map_err(command_error)?;
            Ok(ColdOperation::DeferredOpenOut {
                stream,
                file_name: file_name.packed(),
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CloseOut) => {
            Ok(ColdOperation::DeferredCloseOut {
                stream: processor.scan_write_stream().map_err(command_error)?,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfFontExpand) => {
            // pdftex.web §§1680--1682 configures font metrics independently
            // of the selected output backend; generated fonts are valid in
            // both DVI and PDF mode.
            let font = processor.scan_font_selector().map_err(command_error)?;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let stretch = processor.scan_integer().map_err(command_error)?.value;
            let shrink = processor.scan_integer().map_err(command_error)?.value;
            let step = processor.scan_integer().map_err(command_error)?.value;
            let auto_expand = processor
                .scan_keyword("autoexpand")
                .map_err(command_error)?
                .value;
            let spec =
                tex_typeset::expansion::FontExpansionSpec::new(stretch, shrink, step, auto_expand)?;
            Ok(ColdOperation::PdfFontExpand { font, spec })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfFontAttr
            | UnexpandablePrimitive::PdfIncludeChars
            | UnexpandablePrimitive::PdfMapFile
            | UnexpandablePrimitive::PdfMapLine
            | UnexpandablePrimitive::PdfGlyphToUnicode
            | UnexpandablePrimitive::PdfNoBuiltinToUnicode),
        ) => {
            let dvi_name = match primitive {
                UnexpandablePrimitive::PdfFontAttr => Some("pdffontattr"),
                UnexpandablePrimitive::PdfIncludeChars => Some("pdfincludechars"),
                UnexpandablePrimitive::PdfMapFile => Some("pdfmapfile"),
                UnexpandablePrimitive::PdfMapLine => Some("pdfmapline"),
                _ => None,
            };
            if processor.int_param(IntParam::PDF_OUTPUT) <= 0
                && let Some(name) = dvi_name
            {
                return Err(ExecError::PdfExtensionInDviMode(name));
            }
            let font = matches!(
                primitive,
                UnexpandablePrimitive::PdfFontAttr
                    | UnexpandablePrimitive::PdfIncludeChars
                    | UnexpandablePrimitive::PdfNoBuiltinToUnicode
            )
            .then(|| processor.scan_font_selector().map_err(command_error))
            .transpose()?;
            let first = (!matches!(primitive, UnexpandablePrimitive::PdfNoBuiltinToUnicode))
                .then(|| {
                    processor
                        .scan_balanced_text(true)
                        .map(|text| text.tokens)
                        .map_err(command_error)
                })
                .transpose()?;
            let second = (primitive == UnexpandablePrimitive::PdfGlyphToUnicode)
                .then(|| {
                    processor
                        .scan_balanced_text(true)
                        .map(|text| text.tokens)
                        .map_err(command_error)
                })
                .transpose()?;
            Ok(ColdOperation::PdfFontAction {
                primitive,
                font,
                first,
                second,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Write) => {
            // TeX82 §1350's `new_write_whatsit` normalizes the stream number
            // before storing it in `write_stream(tail)`, for the deferred
            // whatsit exactly as for the `\immediate` one.
            let stream = processor.scan_write_stream().map_err(command_error)?;
            let tokens = processor
                .scan_balanced_text(false)
                .map_err(command_error)?
                .tokens;
            Ok(ColdOperation::DeferredWrite { stream, tokens })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Special) => {
            let (deferred, text) = processor.scan_special().map_err(command_error)?;
            Ok(ColdOperation::DeferredSpecial {
                deferred,
                tokens: text.tokens,
            })
        }
        // TeX82 §1377's `@<Implement \setlanguage@>`, the `set_language_code`
        // limb of §1348's `do_extension`. The mode test comes first and
        // guards the `scan_int`, so the operand is read only when
        // `abs(mode)=hmode` -- horizontal or restricted horizontal here,
        // tex.web's `hmode` and `-hmode`.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SetLanguage) => {
            if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal) {
                Ok(ColdOperation::SetLanguage {
                    language: processor.scan_integer().map_err(command_error)?.value,
                })
            } else {
                Ok(ColdOperation::IllegalSetLanguage {
                    token: command.spelling().semantic_token(),
                })
            }
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::LcCode
            | UnexpandablePrimitive::UcCode
            | UnexpandablePrimitive::SfCode
            | UnexpandablePrimitive::MathCode
            | UnexpandablePrimitive::DelCode),
        ) => {
            // TeX82 §1230 selects the table entry with §434's
            // `scan_char_num`, including its out-of-range recovery to
            // character zero. The assigned value has the table-specific
            // bound below; it is a distinct operand and must not inherit the
            // selector's recovery.
            let character = processor
                .scan_restricted_integer(RestrictedIntegerClass::CharacterCode)
                .map_err(command_error)?
                .value;
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            let character =
                char::from_u32(character as u32).expect("scan_char_num returns a valid character");
            Ok(ColdOperation::CodeTable {
                primitive,
                character,
                value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CatCode) => {
            unreachable!("catcode assignments are owned by fused hot dispatch")
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::PdfLpCode
            | UnexpandablePrimitive::PdfRpCode
            | UnexpandablePrimitive::PdfEfCode
            | UnexpandablePrimitive::PdfTagCode
            | UnexpandablePrimitive::PdfKnbsCode
            | UnexpandablePrimitive::PdfStbsCode
            | UnexpandablePrimitive::PdfShbsCode
            | UnexpandablePrimitive::PdfKnbcCode
            | UnexpandablePrimitive::PdfKnacCode),
        ) => {
            let font = processor.scan_font_selector().map_err(command_error)?;
            let character = processor
                .scan_restricted_integer(RestrictedIntegerClass::CharacterCode)
                .map_err(command_error)?
                .value;
            let character =
                u8::try_from(character).expect("pdfTeX character scanner is byte bounded");
            let _ = processor.scan_optional_equals().map_err(command_error)?;
            let value = processor.scan_integer().map_err(command_error)?.value;
            Ok(ColdOperation::PdfFontCode {
                table: pdf_font_code_table(primitive),
                font,
                character,
                value,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::PdfNoLigatures) => {
            let font = processor.scan_font_selector().map_err(command_error)?;
            Ok(ColdOperation::PdfNoLigatures { font })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Advance
            | UnexpandablePrimitive::Multiply
            | UnexpandablePrimitive::Divide),
        ) => scan_arithmetic_assignment(processor, primitive, global),
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::Def
            | UnexpandablePrimitive::Edef
            | UnexpandablePrimitive::Gdef
            | UnexpandablePrimitive::Xdef,
        ) => unreachable!("macro definitions are owned by fused hot dispatch"),
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::CharDef | UnexpandablePrimitive::MathCharDef),
        ) => {
            // TeX82 §1224 installs the scanner-time `\relax` through
            // `define`, so it has the same effective scope as the eventual
            // definition, including `\globaldefs`. This remains main-control
            // scope policy; the command processor only receives the selected
            // provisional scope while it owns raw operand delivery.
            // §1224's case: `char_def_code` scans §434's `scan_char_num` and
            // `math_char_def_code` scans §436's `scan_fifteen_bit_int`.
            let class = match primitive {
                UnexpandablePrimitive::CharDef => RestrictedIntegerClass::CharacterCode,
                UnexpandablePrimitive::MathCharDef => RestrictedIntegerClass::FifteenBit,
                _ => {
                    unreachable!("outer match restricts primitive to §1224's character shorthands")
                }
            };
            let definition = processor
                .scan_character_definition(class, global)
                .map_err(command_error)?;
            Ok(ColdOperation::CharacterDefinition {
                primitive,
                target: definition.target,
                provisional_old: definition.provisional_old,
                value: definition.value,
                global,
            })
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::CountDef
            | UnexpandablePrimitive::DimenDef
            | UnexpandablePrimitive::SkipDef
            | UnexpandablePrimitive::MuskipDef
            | UnexpandablePrimitive::ToksDef),
        ) => {
            let definition = processor
                .scan_register_definition(global)
                .map_err(command_error)?;
            Ok(ColdOperation::RegisterDefinition {
                primitive,
                target: definition.target,
                provisional_old: definition.provisional_old,
                index: definition.index,
                global,
            })
        }
        // TeX82 §1288's `shift_case` is entirely a command-core operation:
        // `scan_toks`, a `\uccode`/`\lccode` rewrite, and `back_list`. It
        // reaches no stomach state, so it completes inside the command
        // processor and its `back_list` push stays on the observed path.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Uppercase | UnexpandablePrimitive::Lowercase),
        ) => {
            processor
                .shift_case(primitive == UnexpandablePrimitive::Uppercase)
                .map_err(command_error)?;
            Ok(ColdOperation::Continue)
        }
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::Let | UnexpandablePrimitive::FutureLet,
        ) => unreachable!("let assignments are owned by fused hot dispatch"),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::AfterGroup) => {
            Ok(ColdOperation::AfterGroup(
                processor
                    .get_token()
                    .map_err(command_error)?
                    .ok_or(ExecError::MissingToken {
                        context: "\\aftergroup",
                    })?
                    .rooted_spelling(),
            ))
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::AfterAssignment) => {
            Ok(ColdOperation::AfterAssignment(
                processor
                    .get_token()
                    .map_err(command_error)?
                    .ok_or(ExecError::MissingToken {
                        context: "\\afterassignment",
                    })?
                    .spelling()
                    .semantic_token(),
            ))
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Message | UnexpandablePrimitive::ErrMessage),
        ) => {
            let tokens = processor.scan_balanced_text(true).map_err(command_error)?;
            Ok(ColdOperation::Message {
                tokens: tokens.tokens,
                error: primitive == UnexpandablePrimitive::ErrMessage,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Show) => Ok(
            ColdOperation::DisplayDiagnostic(processor.scan_show().map_err(command_error)?),
        ),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowThe) => Ok(
            ColdOperation::DisplayDiagnostic(processor.scan_showthe().map_err(command_error)?),
        ),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowTokens) => {
            let text = processor.scan_showtokens().map_err(command_error)?;
            Ok(ColdOperation::ShowTokens {
                tokens: text.tokens,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowIfs) => {
            Ok(ColdOperation::ShowIfs {
                conditions: processor.active_conditions(),
            })
        }
        // TeX82 §1290's `any_mode(xray): show_whatever` puts every \show
        // family in every mode; §1293's `show_lists_code` case reads no
        // operand at all.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowLists) => {
            Ok(ColdOperation::ShowLists)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowGroups) => {
            Ok(ColdOperation::ShowGroups { diagnostic: None })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ShowBox) => {
            let (index, _) = processor.scan_showbox().map_err(command_error)?;
            Ok(ColdOperation::ShowBox { index })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Immediate) => {
            // TeX82 §§299/367/1370: `write_out` temporarily sets `mode:=0`.
            // The `\immediate` command itself has already consumed this
            // processor episode's main-control prefix, so install `no mode`
            // for an expandable command inside the write text. If one is
            // traced, §367 also leaves `shown_mode=0` after `write_out`
            // restores the real mode.
            let trace_count = processor.command_trace_count();
            processor.set_command_trace_mode_prefix(Some("no mode".into()));
            let extension = processor
                .scan_immediate_extension(processor.int_param(IntParam::PDF_OUTPUT) > 0)
                .map_err(command_error)?;
            if processor.command_trace_count() != trace_count {
                *shown_mode = None;
            }
            if let ImmediateExtension::PdfImage(request) = extension {
                Ok(ColdOperation::PdfXImage {
                    request,
                    resource: PdfImageResource::Unavailable,
                })
            } else {
                Ok(ColdOperation::ImmediateExtension(extension))
            }
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HRule)
            if matches!(mode, Mode::Math | Mode::DisplayMath) =>
        {
            // TeX82 §1046 lists `mmode+hrule` among the "math-only cases in
            // non-math modes, or vice versa"; unlike `mmode+vrule` (§1056,
            // handled below), `\hrule` never reaches `scan_rule_spec` while
            // in math mode. §1047's `insert_dollar_sign` closes math with an
            // inserted `$` and replays `\hrule` in the resulting mode.
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ColdOperation::MissingMathShift)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HRule)
            if mode == Mode::Horizontal =>
        {
            // TeX82 §1095's `head_for_vmode` backs up an `\hrule`, inserts
            // `\par`, and retries the rule in vertical mode. This must happen
            // before §463 scans optional width/height/depth keywords: their
            // lookahead may cross a line boundary, while §804's paragraph
            // diagnostic must retain the line on which the rule was read.
            processor
                .recover_stop_for_vertical_mode(command)
                .map_err(command_error)?;
            Ok(ColdOperation::Continue)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HRule)
            if mode == Mode::RestrictedHorizontal =>
        {
            // TeX82 §1095's negative-mode `head_for_vmode` diagnoses this
            // command immediately. In particular, §463 must not scan a rule
            // specification first: its keyword lookahead would replace the
            // source-line error context with a backed-up token level.
            Ok(ColdOperation::HRuleHereExceptLeaders)
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::HRule | UnexpandablePrimitive::VRule),
        ) => {
            let spec = processor.scan_rule_spec(primitive).map_err(command_error)?;
            Ok(ColdOperation::Rule {
                width: spec.width,
                height: spec.height,
                depth: spec.depth,
                horizontal: primitive == UnexpandablePrimitive::HRule,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::SetBox) => {
            let assignment = processor
                .scan_setbox_assignment(set_box_allowed)
                .map_err(command_error)?;
            Ok(ColdOperation::SetBox {
                target: SetBoxTarget {
                    index: assignment.index,
                    global,
                },
                path: assignment.path,
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VSplit) => Ok(ColdOperation::VSplit(
            processor.scan_vsplit().map_err(command_error)?,
        )),
        // TeX82 §1079's `make_box(box_code)` scans the register through
        // `scan_int` before handing the completed box-list operation to the
        // stomach. In particular, the first digit remains raw command input,
        // never an executor-side backup/replay artifact.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Box | UnexpandablePrimitive::Copy),
        ) => {
            let register = processor.scan_box_register().map_err(command_error)?;
            Ok(ColdOperation::BoxRegister {
                index: register.index,
                copy: primitive == UnexpandablePrimitive::Copy,
                ships_out: boxes.pending_shipout,
            })
        }
        // TeX82 §1095's `hmode+un_vbox: head_for_vmode` ends an unrestricted
        // paragraph and retries the command in vertical mode. As with every
        // `head_for_vmode` command, this happens before `make_box` (§1079)
        // scans the register operand.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::UnVBox
            | UnexpandablePrimitive::UnVCopy
            | UnexpandablePrimitive::PageDiscards
            | UnexpandablePrimitive::SplitDiscards,
        ) if mode == Mode::Horizontal => {
            processor
                .recover_stop_for_vertical_mode(command)
                .map_err(command_error)?;
            Ok(ColdOperation::Continue)
        }
        // The restricted-horizontal branch of §1095 cannot end a paragraph,
        // so it runs §§1064--1066 `off_save`. The recovered command is
        // retried only after the enclosing group has been closed; its
        // register operand must remain unread until that retry.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::UnVBox
            | UnexpandablePrimitive::UnVCopy
            | UnexpandablePrimitive::PageDiscards
            | UnexpandablePrimitive::SplitDiscards,
        ) if mode == Mode::RestrictedHorizontal => {
            scan_off_save(processor, command, innermost_group)
        }
        // e-TeX 2.6 `etex.ch` [15.208, 45.999] assigns both saved-discard
        // enquiries the `un_vbox` command code with modifiers above
        // `copy_code`. TeX82 §1046 consequently routes their math-mode
        // occurrence through `insert_dollar_sign` before `unpackage` can
        // splice the saved list.
        Meaning::UnexpandablePrimitive(
            UnexpandablePrimitive::PageDiscards | UnexpandablePrimitive::SplitDiscards,
        ) if matches!(mode, Mode::Math | Mode::DisplayMath) => {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ColdOperation::MissingMathShift)
        }
        Meaning::UnexpandablePrimitive(
            primitive
            @ (UnexpandablePrimitive::PageDiscards | UnexpandablePrimitive::SplitDiscards),
        ) => Ok(ColdOperation::SavedVerticalDiscards(primitive)),
        // `\unhbox`/`\unhcopy` in (internal) vertical mode never reach here:
        // `starts_paragraph_in_vertical_mode` routes `vmode+un_hbox` through
        // §1090's shared backup above, before this register operand is ever
        // scanned. `\unvbox`/`\unvcopy` are not in that group.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::UnHBox
            | UnexpandablePrimitive::UnHCopy
            | UnexpandablePrimitive::UnVBox
            | UnexpandablePrimitive::UnVCopy),
        ) => {
            let register = processor.scan_box_register().map_err(command_error)?;
            Ok(ColdOperation::Unbox {
                primitive,
                index: register.index,
                error_context: processor.error_context(),
            })
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::LastBox) => {
            Ok(ColdOperation::LastBox {
                error_context: processor.error_context(),
            })
        }
        // TeX82's main-control dispatch on `abs(mode)+cur_cmd` (tex.web
        // §1073): `\raise`/`\lower` (`vmove`) are legal only outside vertical
        // mode (`hmode+vmove`, `mmode+vmove`); `\moveleft`/`\moveright`
        // (`hmove`) are legal only inside it (`vmode+hmove`). The three
        // remaining combinations (`vmode+vmove`, `hmode+hmove`,
        // `mmode+hmove`) are tex.web's "Forbidden cases" list and never
        // reach `scan_normal_dimen` at all -- only `report_illegal_case`
        // fires, so the dimension must not be scanned here.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Raise | UnexpandablePrimitive::Lower),
        ) => {
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                Ok(ColdOperation::IllegalBoxShift {
                    token: command.spelling().semantic_token(),
                })
            } else {
                Ok(ColdOperation::BoxShift(
                    processor.scan_box_shift(primitive).map_err(command_error)?,
                ))
            }
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::MoveLeft | UnexpandablePrimitive::MoveRight),
        ) => {
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                Ok(ColdOperation::BoxShift(
                    processor.scan_box_shift(primitive).map_err(command_error)?,
                ))
            } else {
                Ok(ColdOperation::IllegalBoxShift {
                    token: command.spelling().semantic_token(),
                })
            }
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Leaders
            | UnexpandablePrimitive::CLeaders
            | UnexpandablePrimitive::XLeaders),
        ) => scan_leaders_step(processor, primitive, mode),
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Shipout) => {
            Ok(ColdOperation::BeginShipout)
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::HBox
            | UnexpandablePrimitive::VBox
            | UnexpandablePrimitive::VTop),
        ) => Ok(ColdOperation::BeginBox(
            processor
                .scan_box_construction(primitive)
                .map_err(command_error)?,
        )),
        // TeX82 §1167's `mmode+vcenter`:
        //
        //     mmode+vcenter: begin scan_spec(vcenter_group,false);
        //       normal_paragraph;
        //       push_nest; mode:=-vmode; prev_depth:=ignore_depth;
        //       if every_vbox<>null then
        //         begin_token_list(every_vbox,every_vbox_text);
        //       end;
        //
        // `\vcenter` is a *box* opener, not a math-text field: its body is an
        // internal vertical list built by the same §645 `scan_spec` prefix and
        // the same `push_nest; mode:=-vmode` as `\vbox`, and §1168 packages it
        // with `vpack` before wrapping it in a `vcenter_noad`. Scanning it as
        // a `math_group` field instead (an mlist) silently loses every
        // vertical-mode construction a `\vcenter` body is built from -- above
        // all `\halign`, which §1130 admits only in vertical mode, so plain's
        // `\pmatrix`/`\matrix`/`\cases`/`\eqalign` (all `\vcenter{\ialign{
        // ...}}`) collapsed to their `\mathstrut` alone (`umber2-johp.260`).
        //
        // Outside math mode `\vcenter` never reaches here: §1046's
        // `non_math(vcenter)` sends it through `insert_dollar_sign`, which is
        // the `P::VCenter` arm of the exhaustive fallback below.
        Meaning::UnexpandablePrimitive(primitive @ UnexpandablePrimitive::VCenter)
            if matches!(mode, Mode::Math | Mode::DisplayMath) =>
        {
            Ok(ColdOperation::BeginBox(
                processor
                    .scan_box_construction(primitive)
                    .map_err(command_error)?,
            ))
        }
        // TeX82 §1099's `begin_insert_or_adjust` -- any_mode(insert). `\insert`
        // is legal in every mode with no mode switch of its own, exactly like
        // `\penalty` and `\mark` above.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Insert) => {
            Ok(ColdOperation::BeginInsert(
                processor
                    .scan_insert_construction(false)
                    .map_err(command_error)?,
            ))
        }
        // TeX82 §1099's `begin_insert_or_adjust` with `cur_val:=255` fixed
        // (`if cur_cmd=vadjust then cur_val:=255`) rather than scanned --
        // `\vadjust` shares `\insert`'s exact class-255 body construction,
        // recognized in `finish_insert_or_adjust_group` below. Unlike
        // `\insert`, `\vadjust` is restricted to `hmode+vadjust`/
        // `mmode+vadjust`; `vmode+vadjust` is one of tex.web's "Forbidden
        // cases" (`@<Forbidden...@>=`), so vertical mode never reaches
        // `scan_box_group_opening` at all.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VAdjust) => {
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                Ok(ColdOperation::IllegalInsertOrAdjust {
                    token: command.spelling().semantic_token(),
                })
            } else {
                Ok(ColdOperation::BeginInsert(
                    processor
                        .scan_insert_construction(true)
                        .map_err(command_error)?,
                ))
            }
        }
        // TeX82 §1101's `make_mark` -- any_mode(mark). `p:=scan_toks(false,
        // true)`: a fully expanded balanced general text, exactly like
        // `\special`'s and `\message`'s bodies. Plain `\mark` fixes class
        // zero; the e-TeX numbered variant below scans its class first.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Mark) => Ok(ColdOperation::Mark {
            class: 0,
            tokens: processor
                .scan_balanced_text(true)
                .map_err(command_error)?
                .tokens,
        }),
        // e-TeX 2.6 `etex.ch` [26.424]'s `make_mark`: `\marks` first scans
        // one extended register number (recovering an invalid selector to
        // class zero), then performs TeX82's expanded mark-text scan.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Marks) => {
            let class = processor
                .scan_extended_register_index()
                .map_err(command_error)?;
            Ok(ColdOperation::Mark {
                class,
                tokens: processor
                    .scan_balanced_text(true)
                    .map_err(command_error)?
                    .tokens,
            })
        }
        // TeX82 §1095's `hmode+halign: head_for_vmode` ends an unrestricted
        // paragraph and retries the alignment in vertical mode.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HAlign)
            if mode == Mode::Horizontal =>
        {
            processor
                .recover_stop_for_vertical_mode(command)
                .map_err(command_error)?;
            Ok(ColdOperation::Continue)
        }
        // Restricted horizontal mode cannot end a paragraph, so §§1064--1066
        // close the enclosing group before retrying the same `\halign`.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HAlign)
            if mode == Mode::RestrictedHorizontal =>
        {
            scan_off_save(processor, command, innermost_group)
        }
        // `\halign` is legal directly in vertical mode (TeX82's
        // `vmode+halign:init_align`).
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::HAlign) => {
            if mode == Mode::Math {
                // TeX82 §1130 tests `privileged` before inspecting the
                // current group. Inline math is negative `mmode`, so §1051
                // ignores only the already-delivered command and leaves the
                // following token untouched.
                Ok(ColdOperation::IllegalHAlign {
                    token: command.spelling().semantic_token(),
                })
            } else if mode == Mode::DisplayMath {
                if innermost_group != Some(GroupKind::MathShift) {
                    scan_off_save(processor, command, innermost_group)
                } else {
                    // TeX82 §774's `init_align` admits a display alignment at
                    // the display's own math-shift save level. The execute
                    // phase below diagnoses and flushes any preceding formula
                    // before it opens the alignment list.
                    Ok(ColdOperation::BeginAlignment {
                        vertical: false,
                        owner: command.control_sequence(),
                    })
                }
            } else {
                Ok(ColdOperation::BeginAlignment {
                    vertical: false,
                    owner: command.control_sequence(),
                })
            }
        }
        // Only `hmode+valign` reaches here: §1090 lists `vmode+valign` (unlike
        // `vmode+halign` above), so the shared backup already turned a bare
        // `\valign` in (internal) vertical mode into a paragraph start, and
        // the redelivered token arrives as `hmode+valign` -- embedded
        // alignment material inside the resulting paragraph's horizontal list.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::VAlign) => {
            if matches!(mode, Mode::Math | Mode::DisplayMath)
                && innermost_group != Some(GroupKind::MathShift)
            {
                scan_off_save(processor, command, innermost_group)
            } else {
                Ok(ColdOperation::BeginAlignment {
                    vertical: true,
                    owner: command.control_sequence(),
                })
            }
        }
        // TeX82 §1096: `hmode+par_end` first runs `off_save` when
        // `align_state<0`, then retries the same `\par` after the inserted
        // group closer. A malformed alignment entry can otherwise absorb all
        // following vertical material into its last cell.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par)
            if matches!(mode, Mode::Horizontal | Mode::RestrictedHorizontal)
                && processor.paragraph_end_needs_alignment_recovery() =>
        {
            scan_off_save(processor, command, innermost_group)
        }
        // TeX82 §§1046--1047 classify `mmode+par_end` as a math-mode
        // mismatch: insert `$`, then rescan the same `\par` after the math
        // list has closed. Treating it as an ordinary paragraph terminator
        // leaves the math group open and lets subsequent recovery close
        // unrelated groups instead.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par)
            if matches!(mode, Mode::Math | Mode::DisplayMath) =>
        {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ColdOperation::MissingMathShift)
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Par) => Ok(ColdOperation::Paragraph),
        // TeX82 §1193 closes math only at `math_shift_group`; a `$` inside
        // any nested math group first runs §1064's `off_save`, which inserts
        // that group's required closer and retries this same shift.
        Meaning::CharToken {
            cat: Catcode::MathShift,
            ..
        } if matches!(mode, Mode::Math | Mode::DisplayMath)
            && innermost_group != Some(GroupKind::MathShift) =>
        {
            scan_off_save(processor, command, innermost_group)
        }
        Meaning::CharToken {
            cat: Catcode::MathShift,
            ..
        } => match mode {
            // §1090's shared backup already retried this exact shift after
            // `new_graf`; probing it in vertical mode would run before
            // `\everypar`.
            Mode::Vertical | Mode::InternalVertical => {
                unreachable!("§1090 backs a vertical-mode math shift up first")
            }
            // §1138 `init_math`: `hmode+math_shift`, for either sign of
            // `hmode`. The probe is `get_token`, and only `mode>0` -- the
            // unrestricted horizontal mode -- may consume the second `$`.
            Mode::Horizontal | Mode::RestrictedHorizontal => {
                let paired = processor
                    .scan_init_math_display_pair(mode == Mode::Horizontal)
                    .map_err(command_error)?;
                Ok(ColdOperation::MathShift {
                    pairing: if paired {
                        MathShiftPairing::Paired
                    } else {
                        MathShiftPairing::Unpaired
                    },
                })
            }
            // §1194 `after_math` reaches §1197's `get_x_token` probe twice
            // over: once for a closing display (`m>=0` with `a=null`) and
            // once for a closing equation number (`mode=-m`).
            Mode::DisplayMath => Ok(ColdOperation::MathShift {
                pairing: MathShiftPairing::ProbeDisplayEnd,
            }),
            Mode::Math if display_eq_no => Ok(ColdOperation::MathShift {
                pairing: MathShiftPairing::ProbeDisplayEnd,
            }),
            // §1194's `m<0` closes inline math through `@<Finish math in
            // text@>`, which probes nothing at all.
            Mode::Math => Ok(ColdOperation::MathShift {
                pairing: MathShiftPairing::Unpaired,
            }),
        },
        // §1090's shared backup already handled `vmode+letter` and
        // `vmode+other_char`, so a letter or other character reaching here is
        // in horizontal or math mode. `vmode+spacer` is §1045's `do_nothing`
        // and is the one category code of the three that stays here.
        Meaning::CharToken {
            ch,
            cat: cat @ (Catcode::Letter | Catcode::Other | Catcode::Space),
        } => Ok(ColdOperation::Character {
            ch,
            cat,
            origin: material_origin(processor, &command),
            suppress_left_boundary: false,
        }),
        // TeX82 §1105's `any_mode(remove_item): delete_last`. No operand of
        // its own; `\unpenalty`/`\unkern`/`\unskip` differ only in which node
        // type is a removal target, decided at apply time against the live
        // mode nest and `Universe`.
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::UnPenalty
            | UnexpandablePrimitive::UnKern
            | UnexpandablePrimitive::UnSkip),
        ) => Ok(ColdOperation::DeleteLast {
            primitive,
            context: processor.error_context(),
        }),
        // TeX82 §1111's "Forbidden cases" (`vmode+ital_corr`) vs. §1112's
        // `hmode+ital_corr`/`mmode+ital_corr`. Mode legality is decided here
        // (only `scan_command` sees `command` to back it up before the
        // Forbidden-case diagnostic); the actual append is mode-sensitive
        // apply-time work with no scan of its own.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::ItalicCorrection) => {
            if matches!(mode, Mode::Vertical | Mode::InternalVertical) {
                Ok(ColdOperation::IllegalItalicCorrection {
                    token: command.spelling().semantic_token(),
                })
            } else {
                Ok(ColdOperation::ItalicCorrection)
            }
        }
        // §1090's `vmode+no_boundary` was already backed up above, so only
        // §1030's `hmode+no_boundary` and §1045's `mmode+no_boundary`
        // (`do_nothing`) reach here; both need only the live mode at apply
        // time, with no scan of their own.
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NoBoundary) => {
            Ok(ColdOperation::NoBoundary {
                suppress_right: false,
            })
        }
        // TeX82 §1171's `mmode+non_script` vs. §1046's `non_math(non_script)`
        // recovery, exactly mirroring the `\vskip`-in-math-mode gate above
        // (`recover_missing_math_shift` already implements §1047's
        // `insert_dollar_sign` generically for any command).
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::NonScript) => {
            if matches!(mode, Mode::Math | Mode::DisplayMath) {
                Ok(ColdOperation::NonScript)
            } else {
                processor
                    .recover_missing_math_shift(command)
                    .map_err(command_error)?;
                Ok(ColdOperation::MissingMathShift)
            }
        }
        Meaning::UnexpandablePrimitive(
            primitive @ (UnexpandablePrimitive::Patterns | UnexpandablePrimitive::Hyphenation),
        ) => {
            // TeX82 §960's `new_patterns` (`\patterns`) and §934's
            // `new_hyph_exceptions` (`\hyphenation`) each require §403's
            // `scan_left_brace` and then classify a `get_x_token` loop's
            // deliveries (§961, §935) as word characters, word boundaries, or
            // the closing brace. Neither absorbs a balanced text, so neither
            // enters §473's `absorbing` scanner status.
            //
            // §1252's INITEX-only guard on `\patterns` is applied at the
            // apply seam, not here: its production branch flushes the same
            // braced group (`repeat get_token until cur_cmd=right_brace`)
            // that the scan already consumes, and only the session -- not the
            // command core -- knows which binary tex.web's `init`/`tini`
            // split would have produced.
            let patterns = primitive == UnexpandablePrimitive::Patterns;
            // Captured for both seams before anything of the group is read:
            // the two rejections §1252 can raise each report at this cursor.
            let rejection_context = processor.error_context();
            let trie_built = patterns && !processor.hyphenation_patterns_open();
            if trie_built {
                // TeX82 §960's `trie_not_ready=false` branch diagnoses and
                // discards with `scan_toks(false,false)`. Unlike §961's
                // pattern-word loop, §473 therefore enters `absorbing`
                // before §403 reads the opening brace.
                let _ = processor.scan_balanced_text(false).map_err(command_error)?;
                return Ok(ColdOperation::HyphenationData {
                    words: Vec::new(),
                    pattern_specs: Vec::new(),
                    patterns: true,
                    rejection_context,
                    trie_built,
                });
            }
            let scanned = processor
                .scan_hyphenation_data(if patterns {
                    HyphenationDataKind::Patterns
                } else {
                    HyphenationDataKind::Exceptions
                })
                .map_err(command_error)?;
            Ok(ColdOperation::HyphenationData {
                words: scanned.words,
                pattern_specs: scanned.patterns,
                patterns,
                rejection_context,
                trie_built,
            })
        }
        // Every other `Meaning::UnexpandablePrimitive` reaching this point has
        // no named dispatch arm above (or is legal only in a mode this
        // `command` was not delivered in). `scan_unclassified_primitive` is
        // written as an exhaustive match over `UnexpandablePrimitive`
        // specifically so that a newly added variant fails to compile here
        // instead of silently falling through to a silent
        // `ColdOperation::Continue` -- see umber2-johp.69 and
        // `docs/tex_command_core.md`'s dispatch-completeness invariant.
        Meaning::UnexpandablePrimitive(primitive) => {
            scan_unclassified_primitive(processor, command, primitive, mode)
        }
        // Every other `Meaning` variant reaching this point has no named
        // dispatch arm above. `scan_unclassified_meaning` applies the same
        // remedy one level up the meaning word (umber2-johp.108): it is an
        // exhaustive match over `Meaning` -- and, inside its `CharToken`
        // case, over `Catcode` -- so a newly added variant fails to compile
        // there instead of reaching a silent `ColdOperation::Continue` here.
        meaning => scan_unclassified_meaning(processor, command, meaning, mode, innermost_group),
    }
}

/// Web2C/pdfTeX `partoken.ch` replaces selected direct `end_graf` calls with
/// an inserted `\par` replay only while unrestricted horizontal mode is
/// current. Context one covers vertical boxes; context two adds insertion,
/// output, alignment-item, and no-align boundaries.
pub(in crate::main_control) fn partoken_context_replays(
    processor: &mut CommandProcessor<'_>,
    mode: Mode,
    threshold: i32,
) -> bool {
    mode == Mode::Horizontal && processor.int_param(IntParam::PAR_TOKEN_CONTEXT) >= threshold
}

/// TeX82 §1335 reports and frees unfinished conditionals innermost-first.
pub(in crate::main_control) fn report_incomplete_conditions(
    stores: &mut Universe,
    incomplete: impl IntoIterator<Item = tex_command::IncompleteCondition>,
) {
    let mut printer = stores.printer();
    for condition in incomplete {
        printer
            .print_nl("(")
            .print_esc("end occurred ")
            .print("when ")
            .print_esc(condition.kind_name());
        if condition.source_line() != 0 {
            printer
                .print(" on line ")
                .print_int(i32::try_from(condition.source_line()).unwrap_or(i32::MAX));
        }
        printer.print(" was incomplete)");
    }
}

/// Runs TeX82 §1064's `off_save`, in full generality across every group
/// kind, not just the `RestrictedHorizontal` `\vskip` family that is this
/// function's first caller.
///
/// `off_save` recovers from a command that the current (innermost) group
/// cannot accommodate. Per §1066, a `bottom_level` group (no group open at
/// all) simply drops the command with an "Extra `<command>`" diagnostic --
/// there is nothing to close, so nothing is backed up or replayed. Otherwise
/// §1065 selects one of four closers to insert ahead of the backed-up
/// command, matching `cur_group`: a `semi_simple_group` needs the frozen,
/// redefinition-proof `\endgroup` control sequence (a plain `}` cannot close
/// it); a `math_shift_group` needs `$`; a `math_left_group` needs the
/// two-token `\right.` (frozen `\right` followed by a `.` other-character,
/// mirroring tex.web's `get_avail`-built two-node list); every other group
/// kind (box-making groups among them, the only case reachable from
/// restricted horizontal mode today) needs an ordinary `}`. Selecting and
/// inserting the closer is command-owned
/// (`CommandProcessor::recover_off_save`/`report_off_save_bottom_drop`); the
/// execute phase (`apply_cold_operation`) only prints the matching text once
/// the returned `ColdOperation` is applied.
pub(in crate::main_control) fn scan_off_save(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    innermost_group: Option<GroupKind>,
) -> Result<ColdOperation, ExecError> {
    let Some(kind) = innermost_group else {
        let token = command.spelling().semantic_token();
        processor.report_off_save_bottom_drop(&command);
        return Ok(ColdOperation::OffSaveBottomDrop { token });
    };
    match kind {
        GroupKind::SemiSimple => {
            let endgroup = processor
                .frozen_primitive_token("endgroup")
                .map_err(command_error)?;
            processor
                .recover_off_save(command, &[endgroup])
                .map_err(command_error)?;
            Ok(ColdOperation::OffSave(OffSaveCloser::EndGroup))
        }
        GroupKind::MathShift => {
            let dollar = Token::Char {
                ch: '$',
                cat: Catcode::MathShift,
            };
            processor
                .recover_off_save(command, &[dollar])
                .map_err(command_error)?;
            Ok(ColdOperation::OffSave(OffSaveCloser::MathShift))
        }
        GroupKind::MathLeft => {
            let right = processor
                .frozen_primitive_token("right")
                .map_err(command_error)?;
            let dot = Token::Char {
                ch: '.',
                cat: Catcode::Other,
            };
            processor
                .recover_off_save(command, &[right, dot])
                .map_err(command_error)?;
            Ok(ColdOperation::OffSave(OffSaveCloser::NullRight))
        }
        _ => {
            let right_brace = Token::Char {
                ch: '}',
                cat: Catcode::EndGroup,
            };
            processor
                .recover_off_save(command, &[right_brace])
                .map_err(command_error)?;
            Ok(ColdOperation::OffSave(OffSaveCloser::RightBrace))
        }
    }
}

/// Classifies every `UnexpandablePrimitive` variant that reaches
/// `scan_command`'s final fallback arm.
///
/// This match is deliberately written over the full `UnexpandablePrimitive`
/// enum, not just the ~140 variants that currently lack a dispatch arm
/// above: the `unreachable` bucket below exists so that removing (or
/// mode-narrowing) one of `scan_command`'s existing named arms, or adding a
/// brand new primitive variant, fails to compile until this function is
/// updated with a deliberate decision. This is the mechanism umber2-johp.69
/// asked for: an unimplemented or wrong-mode primitive must stop the run at
/// its true site with a named error, never silently succeed while leaving
/// its own operand tokens (if any) in the input stream to be typeset as
/// literal text -- exactly how umber2-johp.67's `\patterns` bug and
/// umber2-johp.68's `\penalty` bug both escaped detection.
///
/// # Buckets
///
/// - `unreachable!()`: this primitive already has an explicit, mode-complete
///   dispatch arm earlier in `scan_command`'s outer match (including the
///   early math/family gates before that match). It can never actually
///   reach this function; if it does, `scan_command` was edited to narrow or
///   remove that arm without updating this classifier, which is exactly the
///   defect this function exists to catch -- panicking here is preferable to
///   silently reverting to the swallowed-primitive behavior.
/// - `unreachable!()` for the prefixes and `\ignorespaces`: these have no
///   `scan_command` arm at all and must not have one, because tex.web
///   consumes them above its big case (§1211's prefix loop, §1045's
///   `reswitch`). `dispatch_main_control_command` is where that happens, so
///   reaching this function names a caller that bypassed it.
/// - `Err(ExecError::UnimplementedPrimitive { .. })`: this primitive has no
///   dispatch at all yet in main control, or is dispatched only
///   conditionally elsewhere (for example the math-noad family routed
///   through `scan_math_request`, or `\left`/`\right`/`\middle`'s
///   math-delimiter gate) and was reached outside that context, or is a
///   e-TeX/pdfTeX extension whose canonical routing has not been written.
///   Per umber2-johp.69's scope, this function does not implement any of
///   these; it only makes each one fail loudly and names it so follow-on
///   work can be tracked as ordinary chain links (see umber2-johp.74).
/// - `insert_dollar_sign` recovery: this primitive is a member of TeX82
///   §1046's "math-only cases in non-math modes" table (`non_math(...)` in
///   tex.web) -- it is dispatched correctly by `scan_math_request`
///   or the `\left`/`\right`/`\middle` gate above whenever `mode` actually is
///   `Math`/`DisplayMath`, so reaching this function at all proves `mode` is
///   not math. §1047's `insert_dollar_sign` backs the offending command up
///   behind a synthesized `$` (umber2-johp.56/.79's
///   `CommandProcessor::recover_missing_math_shift`, already used by the
///   `mmode+hrule`/`mmode+vskip`/`non_math(non_script)` arms above) so the
///   next two deliveries close math and replay the command in the resulting
///   mode. `\eqno`/`\leqno` are deliberately excluded from this bucket:
///   tex.web's separate `@<Forbidden cases@>=non_math(eq_no)` (§1144) routes
///   them through `report_illegal_case` ("You can't use `\eqno' in ...
///   mode") instead, via their own dedicated `ColdOperation::IllegalEqNo` arm
///   below (umber2-johp.88).
fn scan_unclassified_primitive(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    primitive: UnexpandablePrimitive,
    mode: Mode,
) -> Result<ColdOperation, ExecError> {
    use UnexpandablePrimitive as P;
    match primitive {
        P::Accent
        | P::Advance
        | P::AfterAssignment
        | P::AfterGroup
        | P::BeginGroup
        | P::Box
        | P::CLeaders
        | P::CatCode
        | P::Char
        | P::CharDef
        | P::CloseIn
        | P::CloseOut
        | P::ControlSpace
        | P::Copy
        | P::Count
        | P::CountDef
        | P::Def
        | P::DelCode
        | P::Dimen
        | P::DimenDef
        | P::Discretionary
        | P::Divide
        | P::Dump
        | P::Edef
        | P::End
        | P::EndGroup
        | P::ErrMessage
        | P::Font
        | P::FontDimen
        | P::FutureLet
        | P::Gdef
        | P::HAlign
        | P::HBox
        | P::HFil
        | P::HFilNeg
        | P::HFill
        | P::HRule
        | P::HSkip
        | P::HSs
        | P::HyphenChar
        | P::Hyphenation
        | P::Immediate
        | P::Indent
        | P::Insert
        | P::InteractionMode
        | P::Kern
        | P::LastBox
        | P::LcCode
        | P::Leaders
        | P::Let
        | P::LetterspaceFont
        | P::Lower
        | P::Lowercase
        | P::Marks
        | P::MathCharDef
        | P::MathCode
        | P::Message
        | P::MoveLeft
        | P::MoveRight
        | P::Multiply
        | P::Muskip
        | P::MuskipDef
        | P::NoIndent
        | P::OpenIn
        | P::OpenOut
        | P::Par
        | P::ParShape
        | P::Patterns
        | P::PageDiscards
        | P::PdfAnnot
        | P::PdfCatalog
        | P::PdfColorStack
        | P::PdfDest
        | P::PdfEndLink
        | P::PdfEndThread
        | P::PdfFontAttr
        | P::PdfFontExpand
        | P::PdfGlyphToUnicode
        | P::PdfIncludeChars
        | P::PdfInfo
        | P::PdfLiteral
        | P::PdfCopyFont
        | P::PdfMapFile
        | P::PdfMapLine
        | P::PdfNames
        | P::PdfNoBuiltinToUnicode
        | P::PdfObject
        | P::PdfOutline
        | P::PdfInterwordSpaceOff
        | P::PdfInterwordSpaceOn
        | P::PdfFakeSpace
        | P::PdfRunningLinkOff
        | P::PdfRunningLinkOn
        | P::PdfSpaceFont
        | P::PdfRefXForm
        | P::PdfRefXImage
        | P::PdfReferenceObject
        | P::PdfRestore
        | P::PdfSave
        | P::PdfSavePos
        | P::PdfSnapRefPoint
        | P::PdfSnapY
        | P::PdfSnapYComp
        | P::PdfResetTimer
        | P::PdfSetRandomSeed
        | P::PdfSetMatrix
        | P::PdfStartLink
        | P::PdfStartThread
        | P::PdfThread
        | P::PdfTrailer
        | P::PdfTrailerId
        | P::PdfXForm
        | P::PdfXImage
        | P::ItalicCorrection
        | P::NoBoundary
        | P::NonScript
        | P::Penalty
        | P::PrevDepth
        | P::PrevGraf
        | P::Raise
        | P::Read
        | P::ReadLine
        | P::ScriptFont
        | P::ScriptScriptFont
        | P::SetBox
        | P::SfCode
        | P::Shipout
        | P::Show
        | P::ShowBox
        | P::ShowGroups
        | P::ShowLists
        | P::ShowThe
        | P::ShowTokens
        | P::ShowIfs
        | P::SkewChar
        | P::Skip
        | P::SkipDef
        | P::Special
        | P::TextFont
        | P::Toks
        | P::ToksDef
        | P::UcCode
        | P::UnHBox
        | P::UnHCopy
        | P::UnKern
        | P::UnPenalty
        | P::UnSkip
        | P::UnVBox
        | P::UnVCopy
        | P::SplitDiscards
        | P::Uppercase
        | P::VAlign
        | P::VBox
        | P::VRule
        | P::VSplit
        | P::VTop
        | P::Wd
        | P::Ht
        | P::Dp
        | P::Write
        | P::XLeaders
        | P::Xdef
        | P::Mark
        | P::VAdjust
        | P::SetLanguage
        | P::BatchMode
        | P::ClubPenalties
        | P::DisplayWidowPenalties
        | P::InterLinePenalties
        | P::WidowPenalties
        | P::NonstopMode
        | P::ScrollMode
        | P::QuitVMode
        | P::ErrorStopMode => unreachable!(
            "UnexpandablePrimitive::{primitive:?} has an explicit, mode-complete \
             scan_command dispatch arm and must never reach the exhaustive fallback"
        ),
        // Consumed by `dispatch_main_control_command` *before* the big case,
        // exactly as tex.web consumes them: §1211 `prefixed_command`'s
        // `while cur_cmd=prefix` loop absorbs `\global`/`\long`/`\outer` (and
        // e-TeX's `\protected`) into the accumulator `a` that the assignment
        // cases then read, and §1045's `any_mode(ignore_spaces)` re-enters
        // §1030's `reswitch:` with the next non-blank non-call token. None of
        // them is a mode-dispatched primitive -- §1210 files the prefixes
        // under `any_mode` -- so `scan_command` has, and must have, no arm for
        // them. Reaching this arm means some caller dispatched a command
        // without going through `dispatch_main_control_command`, which is the
        // narrowed-main-control defect of `umber2-johp.208`.
        P::Global | P::Long | P::Outer | P::Protected | P::IgnoreSpaces => unreachable!(
            "UnexpandablePrimitive::{primitive:?} is consumed by \
             dispatch_main_control_command before scan_command; reaching \
             scan_command means a caller bypassed the shared main-control step"
        ),
        // TeX82 §1046's `non_math(...)` table: each of these primitives is a
        // math-noad, math-style, or math-delimiter command whose *only*
        // canonical dispatch is `scan_math_request` (or the
        // `\left`/`\right`/`\middle` gate) under `Mode::Math`/`DisplayMath`.
        // Reaching this arm therefore proves `mode` is not math, which is
        // exactly tex.web's non-math table; §1047's `insert_dollar_sign`
        // recovers uniformly for the whole family via the same
        // `recover_missing_math_shift` helper the `mmode+hrule`/`mmode+vskip`/
        // `non_math(non_script)` arms above already use.
        P::Above
        | P::AboveWithDelims
        | P::Atop
        | P::AtopWithDelims
        | P::Delimiter
        | P::DisplayLimits
        | P::DisplayStyle
        | P::Left
        | P::Limits
        | P::MKern
        | P::MSkip
        | P::MathAccent
        | P::MathBin
        | P::MathChar
        | P::MathChoice
        | P::MathClose
        | P::MathInner
        | P::MathOp
        | P::MathOpen
        | P::MathOrd
        | P::MathPunct
        | P::MathRel
        | P::Middle
        | P::NoLimits
        | P::Over
        | P::OverWithDelims
        | P::Overline
        | P::Radical
        | P::Right
        | P::ScriptScriptStyle
        | P::ScriptStyle
        | P::TextStyle
        | P::Underline
        | P::VCenter => {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ColdOperation::MissingMathShift)
        }
        // TeX82 §1144's `@<Forbidden cases@>=non_math(eq_no)`: unlike the
        // math-noad family immediately above, `\eqno`/`\leqno` outside math
        // mode take `report_illegal_case` ("You can't use `\eqno' in ...
        // mode"), not `insert_dollar_sign` -- tex.web lists them under the
        // separate Forbidden-cases module even though they share the same
        // `eq_no` command code as the math-request vocabulary. Reaching this
        // arm proves `mode` is not `Math`/`DisplayMath` (that gate would
        // have consumed the primitive first via
        // `scan_math_request`'s `Request::EquationNumber`);
        // `mmode+eq_no` itself (gated by `privileged`/`cur_group`) is
        // unaffected.
        P::EqNo | P::LeftEqNo => Ok(ColdOperation::IllegalEqNo {
            token: command.spelling().semantic_token(),
        }),
        // TeX82 §1048's `any_mode(last_item)` Forbidden case: see
        // `ColdOperation::IllegalLastItem`. These internal-only quantities
        // reach this function
        // only when delivered standalone (not mid-scan, where
        // `internal_value_from_command` already consumes them), exactly
        // like `\eqno`/`\leqno` above.
        P::LastKern
        | P::LastPenalty
        | P::LastSkip
        | P::FontCharWd
        | P::FontCharHt
        | P::FontCharDp
        | P::FontCharIc
        | P::ParShapeLength
        | P::ParShapeIndent
        | P::ParShapeDimen
        | P::NumExpr
        | P::DimExpr
        | P::GlueExpr
        | P::MuExpr
        | P::GlueStretch
        | P::GlueShrink
        | P::GlueStretchOrder
        | P::GlueShrinkOrder
        | P::GlueToMu
        | P::MuToGlue => Ok(ColdOperation::IllegalLastItem {
            token: command.spelling().semantic_token(),
            context: processor.error_context(),
        }),
        // TeX82 §1126's `any_mode(car_ret), any_mode(tab_mark): align_error`.
        // `\cr` and `\crcr` carry the `car_ret` command code (chr `cr_code`
        // and `cr_cr_code`); `\span` carries `tab_mark` with chr `span_code`.
        // `get_next` (§342) only diverts them into a v-template when
        // `align_state=0`, so every other occurrence -- inside an alignment
        // cell whose braces are unbalanced, or outside any alignment at all --
        // is main control's to recover through §1127.
        P::Cr | P::CrCr | P::Span => scan_align_error(processor, command),
        // TeX82 §1126's `any_mode(no_align): no_align_error` and
        // `any_mode(omit): omit_error` (§1129). Both routines are a
        // `print_err`/`help2`/`error` triple and nothing else: report the
        // command-specific legal position, then discard the primitive.
        P::NoAlign | P::Omit => Ok(ColdOperation::MisplacedAlignmentCommand {
            omit: primitive == P::Omit,
        }),
        // e-TeX 2.6 `etex.ch` [17.3822--3880] adds four nonzero modifiers to
        // TeX82's `valign` command code. In horizontal mode `eTeX_enabled`
        // first checks `TeXXeT_state>0`; only the enabled branch appends the
        // corresponding zero-width math node. The ordinary zero modifier
        // remains `\valign` and is dispatched above as an alignment.
        P::BeginL | P::BeginR | P::EndL | P::EndR => {
            let direction = match primitive {
                P::BeginL => tex_state::node::Direction::BeginL,
                P::BeginR => tex_state::node::Direction::BeginR,
                P::EndL => tex_state::node::Direction::EndL,
                P::EndR => tex_state::node::Direction::EndR,
                _ => unreachable!("text-direction primitive matched above"),
            };
            Ok(ColdOperation::TextDirection {
                direction,
                enabled: processor.int_param(IntParam::TEX_XET_STATE) > 0,
            })
        }
        P::DiscretionaryHyphen
        | P::GlobalDefs
        | P::PdfEfCode
        | P::PdfKnacCode
        | P::PdfKnbcCode
        | P::PdfKnbsCode
        | P::PdfLpCode
        | P::PdfNoLigatures
        | P::PdfRpCode
        | P::PdfShbsCode
        | P::PdfStbsCode
        | P::PdfTagCode
        | P::SpaceFactor
        | P::VFil
        | P::VFilNeg
        | P::VFill
        | P::VSkip
        | P::VSs => Err(ExecError::UnimplementedPrimitive {
            primitive,
            mode,
            origin: command.origin(),
        }),
    }
}

/// Classifies every `Meaning` variant that `scan_command`'s outer match does
/// not name, so that "no dispatch arm" can never again mean "succeeded and
/// consumed nothing".
///
/// This is `scan_unclassified_primitive`'s sibling one level up the meaning
/// word (umber2-johp.108). That function made the
/// `Meaning::UnexpandablePrimitive` payload compile-time exhaustive, but the
/// outer `Meaning` match kept an ordinary `_ => Ok(ColdOperation::Continue)`
/// wildcard, which became the remaining hiding place: an unrouted meaning
/// left its own operand tokens in the input to be typeset as literal text
/// arbitrarily far from the real defect (umber2-johp.106's `\pagegoal=100pt`
/// is the canonical example). Matching `Meaning` exhaustively here -- and
/// `Catcode` exhaustively inside the `CharToken` case -- converts each such
/// gap into either a deliberate, cited routing decision or a loud, named
/// failure, and makes a newly added variant a build failure.
///
/// # Buckets
///
/// - `Ok(...)`: tex.web routes this meaning somewhere main control
///   already implements generically, cited per arm. Two of these arms
///   reproduce the cited section's *action* while its diagnostic is still
///   missing; both say so and name umber2-johp.110.
/// - `unreachable!()`: `scan_command`'s outer match already has an
///   unconditional named arm for this case, so it cannot arrive here. If it
///   does, that arm was narrowed without updating this classifier, which is
///   exactly the defect this function exists to catch.
/// - `insert_dollar_sign` recovery: this meaning is a member of TeX82
///   §1046's "math-only cases in non-math modes" table (`math_given` and the
///   `sup_mark`/`sub_mark` character categories). Each is dispatched
///   correctly by `scan_command`'s math gates whenever `mode` actually is
///   `Math`/`DisplayMath`, so reaching this function proves `mode` is not
///   math; §1047's `insert_dollar_sign` recovers it through the same
///   `recover_missing_math_shift` the primitive classifier's identical
///   bucket uses.
/// - `Err(ExecError::UnimplementedMeaning { .. })`: main control
///   has no routing for this meaning yet, or the meaning should be
///   unreachable by a gullet invariant and the error names the broken
///   invariant exactly. Per umber2-johp.108's scope this function implements
///   none of them; it only makes each one fail loudly, tracked as
///   umber2-johp.111.
fn scan_unclassified_meaning(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    meaning: Meaning,
    mode: Mode,
    innermost_group: Option<GroupKind>,
) -> Result<ColdOperation, ExecError> {
    match meaning {
        // TeX82 §1045's `any_mode(relax): do_nothing`. `\relax` -- and the
        // frozen relax `\noexpand` substitutes for its operand (§358) -- is
        // the one meaning for which "consume nothing and proceed" is the
        // whole specified behavior.
        Meaning::Relax => Ok(ColdOperation::Relax),
        // TeX82 §1048's Forbidden case `any_mode(last_item)`:
        // `report_illegal_case`. `Meaning::InternalInteger` is tex.web's
        // `last_item` command code with an operand other than
        // `\lastpenalty`/`\lastkern`/`\lastskip` (`\badness`,
        // `\inputlineno`, e-TeX's `\currentgrouplevel` family, pdfTeX's
        // `\pdflastxpos` family, ...). Like those three -- which
        // `scan_unclassified_primitive` already routes to the same
        // `ColdOperation` -- these are legal only as an internal-value operand
        // inside a scan, never as a delivered main-control command.
        Meaning::InternalInteger(_) => Ok(ColdOperation::IllegalLastItem {
            token: command.spelling().semantic_token(),
            context: processor.error_context(),
        }),
        Meaning::CharToken { ch, cat } => {
            scan_unclassified_char_token(processor, command, ch, cat, mode)
        }
        // `scan_command`'s outer match ends with an unconditional
        // `Meaning::UnexpandablePrimitive(primitive)` arm delegating to
        // `scan_unclassified_primitive`, so this payload never reaches here.
        Meaning::UnexpandablePrimitive(_) => {
            unreachable!("unexpandable primitives are classified by scan_unclassified_primitive")
        }
        // TeX82 §1210's `register`, `assign_int`/`assign_dimen`/
        // `assign_glue`/`assign_mu_glue`, `toks_register`/`assign_toks`, and
        // `set_font` assignment forms: `scan_command`'s outer match names
        // every one of them unconditionally.
        Meaning::CountRegister(_)
        | Meaning::DimenRegister(_)
        | Meaning::SkipRegister(_)
        | Meaning::MuskipRegister(_)
        | Meaning::ToksRegister(_)
        | Meaning::IntParam(_)
        | Meaning::DimenParam(_)
        | Meaning::GlueParam(_)
        | Meaning::MuGlueParam(_)
        | Meaning::TokParam(_)
        | Meaning::PageDimension(_)
        | Meaning::PageInteger(_)
        | Meaning::Font(_) => {
            unreachable!("scan_command names this assignment meaning unconditionally")
        }
        // TeX82 gives a `\chardef`'d character exactly the same three-mode
        // behavior as `\char`: §1090's `vmode+char_given` starts a
        // paragraph, §1034's `main_loop` typesets it in horizontal mode, and
        // §1154's `mmode+char_given: set_math_char(ho(math_code(cur_chr)))`
        // appends a math-char noad. tex.web keeps the two interchangeable
        // right down to §1038's ligature lookahead, which accepts
        // `char_given` and `char_num` at the same label, so this reuses
        // `\char`'s own already-dispatched `ColdOperation`; the only
        // difference is that the character code is already known and needs
        // no `scan_char_num`.
        Meaning::CharGiven(ch) => Ok(ColdOperation::CharacterCode {
            value: ch as i32,
            origin: material_origin(processor, &command),
            suppress_left_boundary: false,
        }),
        // TeX82 §1046's `non_math(math_given): insert_dollar_sign`, the same
        // recovery the whole math-only vocabulary takes outside math mode.
        // Reaching this arm proves `mode` is not `Math`/`DisplayMath`:
        // §1154's `mmode+math_given` is dispatched by `scan_command`'s
        // `scan_math_request` gate, which consumes the meaning
        // before its outer match runs.
        Meaning::MathCharGiven(_) => {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ColdOperation::MissingMathShift)
        }
        // TeX82 §§366/370/380 put `undefined_cs` above `max_command`, so the
        // command-owned expanded-delivery loop reports and drops it before
        // main control can receive a command. Reaching stomach dispatch would
        // prove that a fetch policy has recreated a second recovery owner.
        Meaning::Undefined => unreachable!("undefined_cs escaped expanded delivery"),
        // A macro is expanded by `get_x_token` (§380) and `\noexpand` turns
        // one into a frozen relax (§358), so neither should ever be
        // delivered as an unexpandable command. `\endcsname` is the one
        // deliberately unexpandable `ExpandablePrimitive`; TeX82 §1135's
        // `cs_error` gives it "Extra \endcsname", which is not routed here.
        Meaning::ExpandablePrimitive(tex_state::meaning::ExpandablePrimitive::EndCsName) => {
            Ok(ColdOperation::ExtraEndCsName)
        }
        Meaning::Macro { .. } | Meaning::ExpandablePrimitive(_) => {
            Err(unimplemented_meaning(&command, meaning, mode))
        }
        // TeX82 §1130's `vmode+endv,hmode+endv: do_endv` (§1131) and §1046's
        // `mmode+endv: insert_dollar_sign`. `scan_alignment_delivery_step`
        // implements the in-alignment half of §1131 before it ever calls
        // `scan_command`; an `endv` reaching main control by any other route
        // ("a devious user might force an `endv` command to occur just about
        // anywhere", §1131) has no dispatch.
        Meaning::EndV => {
            if matches!(mode, Mode::Math | Mode::DisplayMath) {
                processor
                    .recover_missing_math_shift(command)
                    .map_err(command_error)?;
                Ok(ColdOperation::MissingMathShift)
            } else {
                scan_off_save(processor, command, innermost_group)
            }
        }
        // An opcode `tex-state`'s meaning decoder itself does not recognize.
        Meaning::Unknown(_) => Err(unimplemented_meaning(&command, meaning, mode)),
    }
}

/// Classifies the character-token category codes that `scan_command`'s outer
/// match does not name, exhaustively over [`Catcode`].
///
/// See [`scan_unclassified_meaning`] for the bucket definitions.
fn scan_unclassified_char_token(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
    ch: char,
    cat: Catcode,
    mode: Mode,
) -> Result<ColdOperation, ExecError> {
    match cat {
        // TeX82 §1046's `non_math(sup_mark)`/`non_math(sub_mark)`:
        // §1047's `insert_dollar_sign` backs the command up behind a
        // synthesized `$`. Reaching this arm proves `mode` is not
        // `Math`/`DisplayMath`, since `scan_command`'s superscript/subscript
        // gates consume both categories before its outer match in those two
        // modes.
        Catcode::Superscript | Catcode::Subscript => {
            processor
                .recover_missing_math_shift(command)
                .map_err(command_error)?;
            Ok(ColdOperation::MissingMathShift)
        }
        // TeX82 §1045's `any_mode(mac_param): report_illegal_case`. A bare
        // parameter token has no operand of its own; the command is consumed
        // after the diagnostic and main control continues.
        Catcode::Parameter => Ok(ColdOperation::IllegalMacroParameter {
            token: command.spelling().semantic_token(),
        }),
        // TeX82 §1126's `any_mode(tab_mark)` (a category-4 character token)
        // and `any_mode(car_ret)` (a category-5 one, which `get_next`'s
        // §344 end-of-line handling normally consumes before delivery).
        // Both command codes take §1127's `align_error`.
        Catcode::AlignmentTab | Catcode::EndLine => scan_align_error(processor, command),
        // Category codes that never become a delivered command: `get_next`
        // (§341-§356) consumes escape characters into control-sequence
        // spellings, resolves active characters to their own meanings, drops
        // ignored and comment characters, and reports invalid characters at
        // the lexer boundary.
        Catcode::Escape
        | Catcode::Active
        | Catcode::Ignored
        | Catcode::Comment
        | Catcode::Invalid => Err(unimplemented_meaning(
            &command,
            Meaning::CharToken { ch, cat },
            mode,
        )),
        // `scan_command`'s outer match names all five of these
        // unconditionally.
        Catcode::BeginGroup
        | Catcode::EndGroup
        | Catcode::MathShift
        | Catcode::Space
        | Catcode::Letter
        | Catcode::Other => {
            unreachable!("scan_command names this character category unconditionally")
        }
    }
}

/// TeX82 §1126's `any_mode(car_ret), any_mode(tab_mark): align_error`.
///
/// This is the single entry point for every command tex.web routes to
/// `align_error`: the `car_ret` command code (`\cr`, `\crcr`, and a category-5
/// character token) and the `tab_mark` command code (`\span` and a category-4
/// character token). §1127 chooses between dropping the delimiter (§1128, when
/// `abs(align_state)>2`) and backing it up behind an inserted brace, entirely
/// from the command-owned `align_state`; main control only records whether the
/// inserted brace opens a recovery simple group for §1131's `off_save`.
fn scan_align_error(
    processor: &mut CommandProcessor<'_>,
    command: tex_command::CurrentCommand,
) -> Result<ColdOperation, ExecError> {
    let token = command.spelling().semantic_token();
    match processor
        .recover_align_error(command)
        .map_err(command_error)?
    {
        // TeX82 §1128 calls §82's `error` synchronously, while the delimiter's
        // input level is still current. Split scan/apply must therefore carry
        // that exact context rather than reconstructing it after a retained
        // alignment v-template has advanced or retired.
        None => Ok(ColdOperation::MisplacedAlignmentDelimiter {
            token,
            context: processor.error_context(),
        }),
        Some(tex_state::token::Token::Char {
            cat: brace @ (Catcode::BeginGroup | Catcode::EndGroup),
            ..
        }) => Ok(ColdOperation::AlignmentRecovery { brace }),
        Some(_) => Err(ExecError::MissingToken {
            context: "align_error balancing brace",
        }),
    }
}

fn unimplemented_meaning(
    command: &tex_command::CurrentCommand,
    meaning: Meaning,
    mode: Mode,
) -> ExecError {
    ExecError::UnimplementedMeaning {
        meaning,
        mode,
        origin: command.origin(),
    }
}

/// Scans TeX82's `advance`/`multiply`/`divide` operand sequence wholly
/// through the command processor.  The target's meaning is classified here;
/// application only sees this completed typed description after the processor
/// borrow ends.
fn scan_arithmetic_assignment(
    processor: &mut CommandProcessor<'_>,
    primitive: UnexpandablePrimitive,
    global: bool,
) -> Result<ColdOperation, ExecError> {
    let target_command = processor
        .get_x_token()
        .map_err(command_error)?
        .ok_or(ExecError::UnsupportedAssignmentTarget)?;
    let target = match target_command.meaning() {
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Count) => {
            ArithmeticTarget::IntegerRegister(
                processor
                    .scan_profile_register_index()
                    .map_err(command_error)?,
            )
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Dimen) => {
            ArithmeticTarget::DimensionRegister(
                processor
                    .scan_profile_register_index()
                    .map_err(command_error)?,
            )
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Skip) => {
            ArithmeticTarget::GlueRegister {
                index: processor
                    .scan_profile_register_index()
                    .map_err(command_error)?,
                mu: false,
            }
        }
        Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Muskip) => {
            ArithmeticTarget::GlueRegister {
                index: processor
                    .scan_profile_register_index()
                    .map_err(command_error)?,
                mu: true,
            }
        }
        Meaning::CountRegister(index) => ArithmeticTarget::IntegerRegister(index),
        Meaning::DimenRegister(index) => ArithmeticTarget::DimensionRegister(index),
        Meaning::SkipRegister(index) => ArithmeticTarget::GlueRegister { index, mu: false },
        Meaning::MuskipRegister(index) => ArithmeticTarget::GlueRegister { index, mu: true },
        Meaning::IntParam(index) => ArithmeticTarget::IntegerParameter(index),
        Meaning::DimenParam(index) => ArithmeticTarget::DimensionParameter(index),
        Meaning::GlueParam(index) => ArithmeticTarget::GlueParameter { index, mu: false },
        Meaning::MuGlueParam(index) => ArithmeticTarget::GlueParameter { index, mu: true },
        _ => {
            return Ok(ColdOperation::InvalidArithmeticTarget {
                primitive,
                target: tex_command::PrintCommand::from_current(&target_command),
            });
        }
    };
    let _ = processor.scan_keyword("by").map_err(command_error)?;
    let operand = match target {
        ArithmeticTarget::IntegerRegister(_) | ArithmeticTarget::IntegerParameter(_) => {
            ArithmeticOperand::Integer(processor.scan_integer().map_err(command_error)?.value)
        }
        ArithmeticTarget::DimensionRegister(_) | ArithmeticTarget::DimensionParameter(_) => {
            match primitive {
                UnexpandablePrimitive::Advance => ArithmeticOperand::Dimension(
                    processor.scan_dimension().map_err(command_error)?.value,
                ),
                UnexpandablePrimitive::Multiply | UnexpandablePrimitive::Divide => {
                    ArithmeticOperand::Integer(
                        processor.scan_integer().map_err(command_error)?.value,
                    )
                }
                _ => unreachable!("arithmetic primitive is filtered above"),
            }
        }
        ArithmeticTarget::GlueRegister { mu, .. } | ArithmeticTarget::GlueParameter { mu, .. } => {
            match primitive {
                UnexpandablePrimitive::Advance => {
                    ArithmeticOperand::Glue(processor.scan_glue(mu).map_err(command_error)?.value)
                }
                UnexpandablePrimitive::Multiply | UnexpandablePrimitive::Divide => {
                    ArithmeticOperand::Integer(
                        processor.scan_integer().map_err(command_error)?.value,
                    )
                }
                _ => unreachable!("arithmetic primitive is filtered above"),
            }
        }
    };
    Ok(ColdOperation::Arithmetic {
        primitive,
        target,
        operand,
        global,
    })
}
