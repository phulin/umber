//! Executor-facing structured scanners owned by the command input machine.
//!
//! These wrappers intentionally expose frozen values, provenance, and the
//! canonical filename termination only.  Input levels, raw tokens, and macro
//! argument frames remain private to `tex-command`.

use tex_state::glue::GlueSpec;
use tex_state::interner::Symbol;
use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::scaled::{FontSizeSpec, Scaled};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{SourceId, TracedTokenList};

use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, StoredReplayReason, TokenBehavior,
    TokenPayload, TransientReplayReason,
};
use crate::processor::status::{
    AlignmentId, AlignmentScanContext, ScannerStatus, ScannerWarning, TokenBuilderId,
};
use crate::scan_toks::{ScanToksMode, ScannedToks};
use crate::{
    AlignmentCellTemplates, AlignmentPreamble, CommandError, CommandProcessor, InternalValue,
    processor::{meaning_text, render_the_value, string_text},
};

/// Stable pending-diagnostic identities for TeX82 §760 template recovery.
const MISSING_PARAMETER_DIAGNOSTIC: u64 = 0x616c_6967_0000_0001;
const EXTRA_PARAMETER_DIAGNOSTIC: u64 = 0x616c_6967_0000_0002;

/// Provenance for a completed structured scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StructuredProvenance {
    /// Origin of the first non-ignored token accepted by the scan.
    pub primary: OriginId,
}

/// A balanced token list frozen through the aggregate token store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedBalancedText {
    pub tokens: TracedTokenList,
    pub provenance: StructuredProvenance,
}

/// The two immutable lists collected for a macro definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMacroDefinition {
    /// The raw control-sequence (or active-character) target accepted by
    /// TeX82's `prefixed_command`.  Target delivery is command-owned so the
    /// executor never has to reopen raw input between the primitive and its
    /// parameter/replacement scan.
    pub target: Symbol,
    pub parameter_text: TracedTokenList,
    pub replacement_text: TracedTokenList,
    pub provenance: StructuredProvenance,
    /// TeX82 §1215 substituted the inaccessible recovery target.
    pub missing_target: bool,
    /// TeX82's parameter-text scanner repaired an out-of-order marker.
    pub malformed_parameter: bool,
}

/// A completed TeX82 `\let` or `\futurelet` assignment.
///
/// The command processor owns every raw operand delivery, including the
/// optional equals sign and `\futurelet`'s lookahead replay. Replay receives
/// only the target and its already-resolved source meaning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedLetAssignment {
    pub target: Symbol,
    pub source: Option<Symbol>,
    pub meaning: Meaning,
}

/// TeX82 §§1254--1261's completed `\\font` definition request.
///
/// The target, optional equals, expanded filename, and size clause are all
/// consumed while the command processor is borrowed.  Resource acquisition
/// deliberately happens later through the transient host capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontLoadRequest {
    pub target: Symbol,
    pub name: String,
    pub size: FontSizeSpec,
}

/// The command-owned operand prefix of TeX82's `\setbox` assignment.
///
/// The following box command deliberately remains a normal main-control
/// delivery.  This preserves TeX82's `scan_int`/optional-equals recovery
/// before executor-owned box construction takes over.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedSetBoxAssignment {
    pub index: i32,
}

/// The completed command-owned prefix of a TeX82 box construction.
///
/// `make_box` scans its optional `to`/`spread` clause before it validates the
/// required opening brace.  Keeping both operations here means replay only
/// receives a typed construction request and never needs to reopen input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedBoxConstruction {
    pub kind: ScannedBoxKind,
    pub packing: ScannedPackingSpec,
    /// Whether the required body brace was accepted and backed up for its
    /// ordinary replay delivery. `false` represents TeX's inserted-brace
    /// recovery; the rejected command has already been backed up.
    pub opening_brace_replay: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedBoxKind {
    HBox,
    VBox,
    VTop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedPackingSpec {
    Natural,
    Exactly(Scaled),
    Spread(Scaled),
}

/// The completed register operand of TeX82's `\\box` command.
///
/// `make_box(box_code)` calls `scan_int` before main control can apply the
/// resulting box-list operation. Keeping that scan here preserves the raw
/// digit delivery and any integer-scanner backup entirely in command control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedBoxRegister {
    pub index: i32,
}

/// The complete command-owned operand of TeX82's `\\vsplit`.
///
/// The keyword's absence is preserved so replay can issue its diagnostic, but
/// both the register and dimension have already been consumed canonically.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedVSplit {
    pub index: i32,
    pub height: Scaled,
    pub missing_to: bool,
}

/// A completed display diagnostic. Its text and source origin are frozen while
/// command input is borrowed, leaving replay no operand-reading work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedDisplayDiagnostic {
    pub text: String,
    pub provenance: StructuredProvenance,
}

/// The completed payload prefix of TeX82's `\\leaders` family.
///
/// A constructed box deliberately remains a construction request: its body is
/// replayed through the ordinary box lifecycle before command control scans
/// the following glue operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedLeaderPayload {
    Missing,
    BoxRegister { index: i32, copy: bool },
    Construction(ScannedBoxConstruction),
    Rule(ScannedRuleSpec),
}

/// A completed named glue-parameter assignment.
///
/// Command processing owns the optional equals sign and scalar glue scan;
/// replay receives only the parameter selector and its finished value.  The
/// `mu` flag preserves the TeX distinction between ordinary and math glue
/// parameters without exposing another input path to the executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedGlueParameterAssignment {
    pub index: u16,
    pub value: GlueSpec,
    pub mu: bool,
}

/// A completed TeX82 `\hrule` or `\vrule` specification.
///
/// The command processor owns the expanded `width`, `height`, and `depth`
/// keyword scans and their scalar operands. Replay receives only these final
/// dimensions, so applying a rule cannot open another source-consumption path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedRuleSpec {
    pub width: Option<Scaled>,
    pub height: Option<Scaled>,
    pub depth: Option<Scaled>,
}

/// The character selected as the base of a completed TeX82 `\accent` scan.
///
/// A missing base is deliberately represented explicitly: TeX82 backs the
/// first non-character command up, then inserts the accent by itself.  The
/// command processor owns that backup, so the executor never needs the
/// rejected command or an input cursor to implement this case.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedAccentBase {
    pub character: u8,
    pub provenance: StructuredProvenance,
}

/// Completed command-owned operands for TeX82's text `\accent`.
///
/// The accent code is scanned as an integer, and the following expanded
/// character (including `\char`'s integer operand) is consumed here.  If the
/// next expanded command is not a character, it has already been replayed by
/// the command processor and `base` is `None`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedAccent {
    pub accent: i32,
    pub accent_provenance: StructuredProvenance,
    pub base: Option<ScannedAccentBase>,
}

/// Completed command-owned group material for TeX82's `\discretionary`.
///
/// Each list is an immutable, traced token list.  The group delimiters and
/// all nested token collection stay in command control; a caller may execute
/// each completed list in an isolated restricted-horizontal episode without
/// reopening source input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedDiscretionary {
    pub pre_break: ScannedBalancedText,
    pub post_break: ScannedBalancedText,
    pub replacement: ScannedBalancedText,
}

/// A completed TeX82 math-character operand (`\\mathchar` or `\\mathaccent`).
///
/// The command processor validates the canonical 15-bit range before this
/// crosses the main-control boundary.  Replay therefore has neither an
/// integer scanner nor an invalid-code recovery path for the operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathCharacter {
    pub code: u16,
    pub recovered: bool,
    pub provenance: StructuredProvenance,
}

/// A completed TeX82 delimiter code.  `0` is the canonical missing-delimiter
/// replacement; the diagnostic and rejected-command replay remain command
/// owned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathDelimiter {
    pub code: u32,
    pub recovered: bool,
    pub provenance: StructuredProvenance,
}

/// The font-size bank addressed by a math family assignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathFamilySize {
    Text,
    Script,
    ScriptScript,
}

/// The completed family index prefix of `\\textfont`, `\\scriptfont`, or
/// `\\scriptscriptfont`.  Resolving the following font meaning is deliberately
/// a separate typed operation, so source delivery cannot leak into replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathFamily {
    pub size: MathFamilySize,
    pub family: u8,
    pub recovered: bool,
    pub provenance: StructuredProvenance,
}

/// Placement selected by TeX82's math script controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathScriptKind {
    Subscript,
    Superscript,
}

/// A script marker whose following math field is collected by the canonical
/// math-field episode.  Keeping the marker typed prevents replay from ever
/// reinterpreting `^` or `_` as source tokens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathScript {
    pub kind: MathScriptKind,
    pub provenance: StructuredProvenance,
}

/// Recovery performed while completing a math field or braced math-list
/// episode. The rejected command has already been retained for canonical
/// replay; consumers receive no source cursor or raw command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathEpisodeRecovery {
    None,
    MissingOpeningBrace,
    MissingField,
}

/// Immutable command-owned material for one math field.
///
/// The frozen payload is deliberately private. A consumer can only schedule
/// it through [`CommandState`](crate::CommandState)'s typed replay entry point,
/// keeping command delivery, expansion, and provenance in `tex-command`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathFieldEpisode {
    pub(crate) tokens: TracedTokenList,
    pub recovery: MathEpisodeRecovery,
    pub provenance: StructuredProvenance,
}

/// Immutable command-owned braced mlist episode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathGroupEpisode {
    pub(crate) tokens: TracedTokenList,
    pub recovery: MathEpisodeRecovery,
    pub provenance: StructuredProvenance,
}

/// The four independently frozen branches of TeX82's `\mathchoice`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathChoiceEpisodes {
    pub display: MathGroupEpisode,
    pub text: MathGroupEpisode,
    pub script: MathGroupEpisode,
    pub scriptscript: MathGroupEpisode,
}

/// A completed script attachment. The executor selects the incomplete noad;
/// command processing has already completed the field it attaches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathScriptAttachment {
    pub kind: MathScriptKind,
    pub field: MathFieldEpisode,
}

/// The structural delimiter boundary selected by `\left`, `\right`, or
/// e-TeX's `\middle`. The corresponding delimiter scan is complete before
/// this value crosses the command boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MathDelimiterBoundary {
    pub kind: MathDelimiterBoundaryKind,
    pub delimiter: ScannedMathDelimiter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathDelimiterBoundaryKind {
    Left,
    Right,
    Middle,
}

/// The generalized-fraction form selected before its numerator is frozen.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathFractionKind {
    Over,
    Atop,
    Above,
}

/// Completed command-owned operands of a generalized fraction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedMathFraction {
    pub kind: MathFractionKind,
    pub left_delimiter: Option<ScannedMathDelimiter>,
    pub right_delimiter: Option<ScannedMathDelimiter>,
    pub thickness: Option<Scaled>,
}

/// A completed `\\mskip` or `\\mkern` operand.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScannedMathMuMaterial {
    Glue(GlueSpec),
    Kern(Scaled),
}

/// Which side receives a display equation number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EquationNumberSide {
    Right,
    Left,
}

/// Immutable entry request for `\\eqno` and `\\leqno`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScannedEquationNumber {
    pub side: EquationNumberSide,
}

/// The noad constructor selected by a math-text primitive. Its field is
/// completed by the dedicated canonical math-field episode, not by executor
/// source reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathTextFieldKind {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
    Underline,
    Overline,
    VCenter,
}

/// Immutable request kinds delivered from command processing to canonical main
/// control for TeX82 §§691–734.  Variants that introduce an mlist episode
/// deliberately contain no source cursor: the later stomach migration can
/// consume only the already-classified request and ask the same processor for
/// the next completed field/group episode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CanonicalMathRequest {
    Character(ScannedMathCharacter),
    Family(ScannedMathFamily),
    TextField(MathTextFieldKind),
    Script(ScannedMathScript),
    Limits(MathLimitKind),
    Fraction(ScannedMathFraction),
    Style(MathStyleKind),
    Choice,
    Delimiter(ScannedMathDelimiter),
    Radical(ScannedMathDelimiter),
    Accent(ScannedMathCharacter),
    MuMaterial(ScannedMathMuMaterial),
    EquationNumber(ScannedEquationNumber),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathLimitKind {
    Limits,
    NoLimits,
    DisplayLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MathStyleKind {
    Display,
    Text,
    Script,
    ScriptScript,
}

/// The canonical boundary that stopped an unbraced filename scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileNameTermination {
    Group,
    Space,
    NonCharacter,
    EndOfInput,
}

/// A filename scanned from expanded command-owned input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScannedFileName {
    pub name: String,
    pub termination: FileNameTermination,
    pub provenance: StructuredProvenance,
}

/// Completed input-stream operation.  The command core owns every operand;
/// replay only acquires an already-registered immutable resource and mutates
/// World stream state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputStreamRequest {
    Open {
        stream: i32,
        file_name: ScannedFileName,
    },
    Close {
        stream: i32,
    },
    Read {
        stream: i32,
        target: Symbol,
        raw_catcodes: bool,
    },
}

/// A completed TeX82 §53 `\immediate` extension request.
///
/// Command control owns the recursive expanded lookahead and all operand
/// scanning.  In particular, a non-I/O lookahead has already been backed up
/// when `Continue` is returned, so replay never needs raw input access.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImmediateExtension {
    Continue,
    OpenOut {
        stream: i32,
        file_name: ScannedFileName,
    },
    Write {
        stream: i32,
        tokens: TracedTokenList,
    },
    CloseOut {
        stream: i32,
    },
}

/// One successfully opened capability-registered input source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredInput {
    pub file_name: ScannedFileName,
    pub source: SourceId,
}

/// The typed result of TeX82's `init_col` entry lookahead.
///
/// `\omit` is consumed as that lookahead rather than backed up for the
/// selected u-template. The executor receives this semantic distinction,
/// never the command spelling that established it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignmentCellOpening {
    /// The selected column uses its ordinary u-template.
    Template,
    /// The selected column starts with TeX82's template-free `\omit` path.
    Omit,
}

impl CommandProcessor<'_> {
    /// Completes one TeX82 math field into immutable command-owned material.
    ///
    /// A braced field becomes a grouped mlist episode; an unbraced field is
    /// one expanded command spelling. Active characters are resolved by
    /// `get_x_token` before freezing, so their replay cannot reopen source
    /// input or bypass command provenance.
    pub fn scan_math_field_episode(&mut self) -> Result<MathFieldEpisode, CommandError> {
        loop {
            let Some(command) = self.get_x_token()? else {
                return Ok(MathFieldEpisode {
                    tokens: self.state.finish_traced_token_list(&[]),
                    recovery: MathEpisodeRecovery::MissingField,
                    provenance: StructuredProvenance {
                        primary: OriginId::UNKNOWN,
                    },
                });
            };
            match command.meaning() {
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
                | Meaning::Relax => continue,
                Meaning::CharToken {
                    cat: Catcode::BeginGroup,
                    ..
                } => {
                    self.back_input(command)?;
                    let group = self.scan_math_group_episode()?;
                    return Ok(MathFieldEpisode {
                        tokens: group.tokens,
                        recovery: group.recovery,
                        provenance: group.provenance,
                    });
                }
                _ => {
                    let provenance = StructuredProvenance {
                        primary: command.origin(),
                    };
                    return Ok(MathFieldEpisode {
                        tokens: self.state.finish_traced_token_list(&[command.spelling()]),
                        recovery: MathEpisodeRecovery::None,
                        provenance,
                    });
                }
            }
        }
    }

    /// Completes one required braced math list. A missing opening brace is
    /// recovered here after `scan_left_brace` has backed the rejected command
    /// up, matching TeX's recovery ownership without exposing it to replay.
    pub fn scan_math_group_episode(&mut self) -> Result<MathGroupEpisode, CommandError> {
        match self.scan_left_brace(true) {
            Ok(opening) => {
                let primary = opening.origin();
                self.back_input(opening)?;
                let scanned = self.scan_toks(ScanToksMode::General { expanded: false })?;
                Ok(MathGroupEpisode {
                    tokens: scanned.replacement_text,
                    recovery: MathEpisodeRecovery::None,
                    provenance: StructuredProvenance { primary },
                })
            }
            Err(CommandError::InputInvariant) => Ok(MathGroupEpisode {
                tokens: self.state.finish_traced_token_list(&[]),
                recovery: MathEpisodeRecovery::MissingOpeningBrace,
                provenance: StructuredProvenance {
                    primary: OriginId::UNKNOWN,
                },
            }),
            Err(error) => Err(error),
        }
    }

    /// Completes all four required `\mathchoice` groups before replay starts
    /// constructing any branch.
    pub fn scan_math_choice_episodes(&mut self) -> Result<MathChoiceEpisodes, CommandError> {
        Ok(MathChoiceEpisodes {
            display: self.scan_math_group_episode()?,
            text: self.scan_math_group_episode()?,
            script: self.scan_math_group_episode()?,
            scriptscript: self.scan_math_group_episode()?,
        })
    }

    /// Completes a script marker and its field in one command-owned episode.
    pub fn scan_math_script_attachment(
        &mut self,
        kind: MathScriptKind,
    ) -> Result<MathScriptAttachment, CommandError> {
        Ok(MathScriptAttachment {
            kind,
            field: self.scan_math_field_episode()?,
        })
    }

    /// Completes the delimiter immediately following a structural math
    /// boundary (`\left`, `\right`, or `\middle`).
    pub fn scan_math_delimiter_boundary(
        &mut self,
        kind: MathDelimiterBoundaryKind,
    ) -> Result<MathDelimiterBoundary, CommandError> {
        Ok(MathDelimiterBoundary {
            kind,
            delimiter: self.scan_math_delimiter()?,
        })
    }

    /// Scans and range-checks TeX82's 15-bit math-character number.
    pub fn scan_math_character(&mut self) -> Result<ScannedMathCharacter, CommandError> {
        let scanned = self.scan_integer()?;
        let recovered = !(0..=0x7fff).contains(&scanned.value);
        Ok(ScannedMathCharacter {
            code: if recovered { 0 } else { scanned.value as u16 },
            recovered,
            provenance: StructuredProvenance {
                primary: scanned.provenance.primary,
            },
        })
    }

    /// Scans and range-checks TeX82's 27-bit delimiter number.
    pub fn scan_math_delimiter(&mut self) -> Result<ScannedMathDelimiter, CommandError> {
        let scanned = self.scan_integer()?;
        let recovered = !(0..=0x07ff_ffff).contains(&scanned.value);
        Ok(ScannedMathDelimiter {
            code: if recovered { 0 } else { scanned.value as u32 },
            recovered,
            provenance: StructuredProvenance {
                primary: scanned.provenance.primary,
            },
        })
    }

    /// Scans the family index prefix common to TeX82's three math-font
    /// assignment primitives. Out-of-range families recover to family zero;
    /// the later font-meaning scan is intentionally not part of this request.
    pub fn scan_math_family(
        &mut self,
        size: MathFamilySize,
    ) -> Result<ScannedMathFamily, CommandError> {
        let scanned = self.scan_integer()?;
        let family = u8::try_from(scanned.value).ok().filter(|value| *value < 16);
        Ok(ScannedMathFamily {
            size,
            family: family.unwrap_or(0),
            recovered: family.is_none(),
            provenance: StructuredProvenance {
                primary: scanned.provenance.primary,
            },
        })
    }

    /// Collects the command-owned scalar prefix of TeX82's generalized
    /// fraction forms. Numerator/denominator mlist construction stays in the
    /// executor and is deliberately absent from this scanner boundary.
    pub fn scan_math_fraction(
        &mut self,
        kind: MathFractionKind,
        with_delimiters: bool,
    ) -> Result<ScannedMathFraction, CommandError> {
        let (left_delimiter, right_delimiter) = if with_delimiters {
            (
                Some(self.scan_math_delimiter()?),
                Some(self.scan_math_delimiter()?),
            )
        } else {
            (None, None)
        };
        let thickness = match kind {
            MathFractionKind::Above => Some(self.scan_dimension()?.value),
            MathFractionKind::Atop => Some(Scaled::from_raw(0)),
            MathFractionKind::Over => None,
        };
        Ok(ScannedMathFraction {
            kind,
            left_delimiter,
            right_delimiter,
            thickness,
        })
    }

    /// Scans the `mu`-unit material operands used only in math mode.
    pub fn scan_math_mu_material(
        &mut self,
        glue: bool,
    ) -> Result<ScannedMathMuMaterial, CommandError> {
        if glue {
            Ok(ScannedMathMuMaterial::Glue(self.scan_glue(true)?.value))
        } else {
            Ok(ScannedMathMuMaterial::Kern(self.scan_mu_dimension()?.value))
        }
    }

    /// Completes the scalar portion of a math-mode primitive.  Any following
    /// field or braced list is intentionally represented by a later opaque
    /// replay episode, so the stomach never receives a source cursor.
    pub fn scan_canonical_math_request(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<Option<CanonicalMathRequest>, CommandError> {
        use CanonicalMathRequest as Request;
        use MathTextFieldKind as Field;
        let request = match primitive {
            UnexpandablePrimitive::MathChar => Request::Character(self.scan_math_character()?),
            UnexpandablePrimitive::Delimiter => Request::Delimiter(self.scan_math_delimiter()?),
            UnexpandablePrimitive::MathOrd => Request::TextField(Field::Ord),
            UnexpandablePrimitive::MathOp => Request::TextField(Field::Op),
            UnexpandablePrimitive::MathBin => Request::TextField(Field::Bin),
            UnexpandablePrimitive::MathRel => Request::TextField(Field::Rel),
            UnexpandablePrimitive::MathOpen => Request::TextField(Field::Open),
            UnexpandablePrimitive::MathClose => Request::TextField(Field::Close),
            UnexpandablePrimitive::MathPunct => Request::TextField(Field::Punct),
            UnexpandablePrimitive::MathInner => Request::TextField(Field::Inner),
            UnexpandablePrimitive::Underline => Request::TextField(Field::Underline),
            UnexpandablePrimitive::Overline => Request::TextField(Field::Overline),
            UnexpandablePrimitive::VCenter => Request::TextField(Field::VCenter),
            UnexpandablePrimitive::Limits => Request::Limits(MathLimitKind::Limits),
            UnexpandablePrimitive::NoLimits => Request::Limits(MathLimitKind::NoLimits),
            UnexpandablePrimitive::DisplayLimits => Request::Limits(MathLimitKind::DisplayLimits),
            UnexpandablePrimitive::Over => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Over, false)?)
            }
            UnexpandablePrimitive::Atop => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Atop, false)?)
            }
            UnexpandablePrimitive::Above => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Above, false)?)
            }
            UnexpandablePrimitive::OverWithDelims => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Over, true)?)
            }
            UnexpandablePrimitive::AtopWithDelims => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Atop, true)?)
            }
            UnexpandablePrimitive::AboveWithDelims => {
                Request::Fraction(self.scan_math_fraction(MathFractionKind::Above, true)?)
            }
            UnexpandablePrimitive::Radical => Request::Radical(self.scan_math_delimiter()?),
            UnexpandablePrimitive::Accent | UnexpandablePrimitive::MathAccent => {
                Request::Accent(self.scan_math_character()?)
            }
            UnexpandablePrimitive::MSkip => Request::MuMaterial(self.scan_math_mu_material(true)?),
            UnexpandablePrimitive::MKern => Request::MuMaterial(self.scan_math_mu_material(false)?),
            UnexpandablePrimitive::MathChoice => Request::Choice,
            UnexpandablePrimitive::DisplayStyle => Request::Style(MathStyleKind::Display),
            UnexpandablePrimitive::TextStyle => Request::Style(MathStyleKind::Text),
            UnexpandablePrimitive::ScriptStyle => Request::Style(MathStyleKind::Script),
            UnexpandablePrimitive::ScriptScriptStyle => Request::Style(MathStyleKind::ScriptScript),
            UnexpandablePrimitive::EqNo => Request::EquationNumber(ScannedEquationNumber {
                side: EquationNumberSide::Right,
            }),
            UnexpandablePrimitive::LeftEqNo => Request::EquationNumber(ScannedEquationNumber {
                side: EquationNumberSide::Left,
            }),
            _ => return Ok(None),
        };
        Ok(Some(request))
    }

    /// Scans TeX82's `\\openin`, `\\closein`, `\\read`, and e-TeX's
    /// `\\readline` operands without exposing a raw delivery to replay.
    pub fn scan_input_stream_request(
        &mut self,
        primitive: tex_state::meaning::UnexpandablePrimitive,
    ) -> Result<InputStreamRequest, CommandError> {
        use tex_state::meaning::UnexpandablePrimitive;

        let stream = self.scan_integer()?.value;
        match primitive {
            UnexpandablePrimitive::OpenIn => {
                let _ = self.scan_optional_equals()?;
                Ok(InputStreamRequest::Open {
                    stream,
                    file_name: self.scan_file_name()?,
                })
            }
            UnexpandablePrimitive::CloseIn => Ok(InputStreamRequest::Close { stream }),
            UnexpandablePrimitive::Read | UnexpandablePrimitive::ReadLine => {
                if !self.scan_keyword("to")?.value {
                    return Err(CommandError::InputInvariant);
                }
                let target = self
                    .next_non_space_raw()?
                    .and_then(|command| command.control_sequence())
                    .ok_or(CommandError::InputInvariant)?;
                Ok(InputStreamRequest::Read {
                    stream,
                    target,
                    raw_catcodes: primitive == UnexpandablePrimitive::ReadLine,
                })
            }
            _ => Err(CommandError::InputInvariant),
        }
    }

    /// Scans a complete ordinary font definition without retaining a raw
    /// command, cursor, or host capability.
    pub fn scan_font_definition(&mut self) -> Result<FontLoadRequest, CommandError> {
        let target = self
            .next_non_space_raw()?
            .and_then(|command| command.control_sequence())
            .ok_or(CommandError::InputInvariant)?;
        let _ = self.scan_optional_equals()?;
        let file_name = self.scan_file_name()?;
        let size = if self.scan_keyword("at")?.value {
            let requested = self.scan_dimension()?.value;
            FontSizeSpec::At(
                if requested.raw() > 0 && requested.raw() < 2048 * Scaled::UNITY {
                    requested
                } else {
                    Scaled::from_raw(10 * Scaled::UNITY)
                },
            )
        } else if self.scan_keyword("scaled")?.value {
            let requested = self.scan_integer()?.value;
            FontSizeSpec::Scale(if (1..=32_768).contains(&requested) {
                requested
            } else {
                1000
            })
        } else {
            FontSizeSpec::Design
        };
        Ok(FontLoadRequest {
            target,
            name: file_name.name,
            size,
        })
    }
    /// Scans TeX82 §1124's text-accent operands through command-owned input.
    ///
    /// Assignment execution between the accent code and base character is an
    /// executor lifecycle concern; this bounded scanner intentionally owns
    /// only expanded delivery, `\char`'s scalar operand, and the canonical
    /// replay of a non-character lookahead.
    pub fn scan_accent(&mut self) -> Result<ScannedAccent, CommandError> {
        let accent = self.scan_integer()?;
        let base = loop {
            let Some(command) = self.get_x_token()? else {
                break None;
            };
            match command.meaning() {
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
                | Meaning::Relax => continue,
                Meaning::CharToken {
                    ch,
                    cat: Catcode::Letter | Catcode::Other,
                }
                | Meaning::CharGiven(ch)
                | Meaning::CharToken {
                    ch,
                    cat: Catcode::Active,
                } => {
                    let character =
                        u8::try_from(ch as u32).map_err(|_| CommandError::InputInvariant)?;
                    break Some(ScannedAccentBase {
                        character,
                        provenance: StructuredProvenance {
                            primary: command.origin(),
                        },
                    });
                }
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Char) => {
                    let character = u8::try_from(self.scan_integer()?.value)
                        .map_err(|_| CommandError::InputInvariant)?;
                    break Some(ScannedAccentBase {
                        character,
                        provenance: StructuredProvenance {
                            primary: command.origin(),
                        },
                    });
                }
                _ => {
                    self.back_input(command)?;
                    break None;
                }
            }
        };
        Ok(ScannedAccent {
            accent: accent.value,
            accent_provenance: StructuredProvenance {
                primary: accent.provenance.primary,
            },
            base,
        })
    }

    /// Collects all three TeX82 `\discretionary` groups as immutable traced
    /// material. Their eventual restricted-horizontal execution is separate
    /// from raw source collection and cannot access an `InputStack`.
    pub fn scan_discretionary(&mut self) -> Result<ScannedDiscretionary, CommandError> {
        Ok(ScannedDiscretionary {
            pre_break: self.scan_balanced_text(false)?,
            post_break: self.scan_balanced_text(false)?,
            replacement: self.scan_balanced_text(false)?,
        })
    }

    /// Scans TeX82 §53's one-token `\immediate` extension execution.
    ///
    /// `do_extension` calls `get_x_token`, executes only `openout`, `write`,
    /// and `closeout`, and backs every other expanded command up for ordinary
    /// main control.  The integer, optional-equals, filename, and write-text
    /// scans remain in this command-owned episode.
    pub fn scan_immediate_extension(&mut self) -> Result<ImmediateExtension, CommandError> {
        let command = loop {
            let command = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
            if !matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                break command;
            }
        };
        match command.meaning() {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::OpenOut) => {
                let stream = self.scan_integer()?.value;
                let _ = self.scan_optional_equals()?;
                let file_name = self.scan_file_name()?;
                Ok(ImmediateExtension::OpenOut { stream, file_name })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Write) => {
                let stream = self.scan_integer()?.value;
                // TeX82 §53 first saves write text without expansion, then
                // `write_out` replays it under an outer `\\endwrite` stopper
                // and scans the resulting expanded text. Keep both episodes
                // command-owned; replay receives only the frozen result.
                let tokens = self.scan_immediate_write_text()?;
                let tokens = self.expand_write_text(tokens)?;
                Ok(ImmediateExtension::Write { stream, tokens })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::CloseOut) => {
                let stream = self.scan_integer()?.value;
                Ok(ImmediateExtension::CloseOut { stream })
            }
            _ => {
                self.back_input(command)?;
                Ok(ImmediateExtension::Continue)
            }
        }
    }

    fn expand_write_text(
        &mut self,
        tokens: TracedTokenList,
    ) -> Result<TracedTokenList, CommandError> {
        let endwrite = self
            .state
            .primitive_token("endwrite")
            .ok_or(CommandError::InputInvariant)?;
        let right_brace = Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        };
        let left_brace = Token::Char {
            ch: '{',
            cat: Catcode::BeginGroup,
        };

        // The bottom stopper delivers the synthetic closing brace followed
        // by frozen outer `\\endwrite`; the write list and opening brace sit
        // above it exactly as TeX82's three `ins_list` calls do.
        self.push_write_recovery(vec![right_brace, endwrite], right_brace);
        let write_level = self.command.push_token_level(
            TokenPayload::Stored {
                tokens: tokens.token_list(),
                origins: tokens.origin_list(),
            },
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::Stored(StoredReplayReason::Write),
        );
        #[cfg(any(test, feature = "instrumentation"))]
        {
            // §53 names this artificial replay as a write input lifetime.
            // Keep that observer classification at the scanner/control seam;
            // the raw delivery loop must not inspect the level's trace.
            self.observe_immediate_write_retirement(write_level);
            self.observe(crate::CommandObservation::Input(crate::InputRecord {
                transition: crate::InputTransition::Push,
                reason: crate::InputReason::Write,
                level: write_level.0,
                position: 0,
            }));
        }
        self.push_write_recovery(vec![left_brace], left_brace);

        let expanded = self.scan_balanced_text(true)?.tokens;
        let stopper = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        if stopper.spelling().semantic_token() != endwrite {
            return Err(CommandError::InputInvariant);
        }
        self.retire_last_delivery_level()?;
        Ok(expanded)
    }

    /// Freezes the ordinary `\\write` text after TeX82's `scan_int`
    /// terminator has been validated and backed up. Unlike general-text
    /// callers, §53's `new_write_whatsit` enters the absorbing collection at
    /// that already-backed-up brace.
    fn scan_immediate_write_text(&mut self) -> Result<TracedTokenList, CommandError> {
        let scanned = self.scan_toks(ScanToksMode::GeneralAfterOpening {
            expanded: false,
            primary: OriginId::UNKNOWN,
        })?;
        Ok(scanned.replacement_text)
    }

    fn push_write_recovery(&mut self, tokens: Vec<Token>, observed: Token) {
        let level = self.command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(
                tokens
                    .into_iter()
                    .map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN))
                    .collect::<Vec<_>>(),
            )),
            TokenBehavior::Recovery,
            RetirementBehavior::Pop,
            ReplayTrace::Transient(TransientReplayReason::Inserted),
        );
        #[cfg(any(test, feature = "instrumentation"))]
        {
            self.observe(crate::CommandObservation::Input(crate::InputRecord {
                transition: crate::InputTransition::Recovery,
                reason: crate::InputReason::Recovery,
                level: level.0,
                position: 0,
            }));
            self.observe(crate::CommandObservation::Recovery(crate::RecoveryRecord {
                kind: crate::RecoveryKind::InsertedToken,
                tokens: vec![
                    self.observed_token(TracedTokenWord::pack(observed, OriginId::UNKNOWN)),
                ],
            }));
        }
    }

    /// Scans the register number and optional equals sign of `\setbox`.
    ///
    /// TeX.web's `prefixed_command` dispatches `set_box` to `scan_int` then
    /// `scan_optional_equals`; the latter must retain its ordinary backup
    /// transition when the equals sign is present.
    pub fn scan_setbox_assignment(&mut self) -> Result<ScannedSetBoxAssignment, CommandError> {
        let index = self.scan_integer()?.value;
        let _ = self.scan_optional_equals()?;
        Ok(ScannedSetBoxAssignment { index })
    }

    /// Scans the register operand of TeX82 §1071's `make_box(box_code)`.
    pub fn scan_box_register(&mut self) -> Result<ScannedBoxRegister, CommandError> {
        Ok(ScannedBoxRegister {
            index: self.scan_integer()?.value,
        })
    }

    /// Scans TeX82 §§1076--1081's `\\vsplit <number> to <dimen>` prefix.
    pub fn scan_vsplit(&mut self) -> Result<ScannedVSplit, CommandError> {
        let index = self.scan_integer()?.value;
        let missing_to = !self.scan_keyword("to")?.value;
        let height = self.scan_dimension()?.value;
        Ok(ScannedVSplit {
            index,
            height,
            missing_to,
        })
    }

    /// TeX82 §46's raw `\\show` operand scan.
    pub fn scan_show(&mut self) -> Result<ScannedDisplayDiagnostic, CommandError> {
        let command = self.get_token()?.ok_or(CommandError::InputInvariant)?;
        let token = command.spelling().semantic_token();
        let text = match token {
            Token::Cs(_)
            | Token::Char {
                cat: Catcode::Active,
                ..
            } => format!(
                "\n> {}={}.\n",
                string_text(&self.state, token),
                meaning_text(&self.state, &command)
            ),
            Token::Char { .. } | Token::Param(_) | Token::Frozen(_) => {
                format!("\n> {}.\n", meaning_text(&self.state, &command))
            }
        };
        Ok(ScannedDisplayDiagnostic {
            text,
            provenance: StructuredProvenance {
                primary: command.origin(),
            },
        })
    }

    /// TeX82 §46's `\\showthe` internal-value scan.
    pub fn scan_showthe(&mut self) -> Result<ScannedDisplayDiagnostic, CommandError> {
        let value = self
            .scan_internal_value()?
            .ok_or(CommandError::InputInvariant)?;
        let text = match value.value {
            value @ (InternalValue::Integer(_)
            | InternalValue::Dimension(_)
            | InternalValue::Glue(_)
            | InternalValue::MuGlue(_)) => {
                render_the_value(value).expect("non-token values render")
            }
            InternalValue::Tokens { tokens, .. } => self
                .state
                .tokens(tokens)
                .iter()
                .copied()
                .map(|token| string_text(&self.state, token))
                .collect(),
        };
        Ok(ScannedDisplayDiagnostic {
            text: format!("\n> {text}.\n"),
            provenance: StructuredProvenance {
                primary: value.provenance.primary,
            },
        })
    }

    /// TeX82 §46's expanded box-register scan for `\\showbox`.
    pub fn scan_showbox(&mut self) -> Result<(i32, StructuredProvenance), CommandError> {
        let index = self.scan_integer()?;
        Ok((
            index.value,
            StructuredProvenance {
                primary: index.provenance.primary,
            },
        ))
    }

    /// Scans the payload prefix of TeX82 §1090's leader commands.
    pub fn scan_leader_payload(&mut self) -> Result<ScannedLeaderPayload, CommandError> {
        let Some(command) = self.get_x_token()? else {
            return Ok(ScannedLeaderPayload::Missing);
        };
        match command.meaning() {
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Box) => {
                Ok(ScannedLeaderPayload::BoxRegister {
                    index: self.scan_integer()?.value,
                    copy: false,
                })
            }
            Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Copy) => {
                Ok(ScannedLeaderPayload::BoxRegister {
                    index: self.scan_integer()?.value,
                    copy: true,
                })
            }
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::HBox
                | UnexpandablePrimitive::VBox
                | UnexpandablePrimitive::VTop),
            ) => Ok(ScannedLeaderPayload::Construction(
                self.scan_box_construction(primitive)?,
            )),
            Meaning::UnexpandablePrimitive(
                primitive @ (UnexpandablePrimitive::HRule | UnexpandablePrimitive::VRule),
            ) => Ok(ScannedLeaderPayload::Rule(self.scan_rule_spec(primitive)?)),
            _ => {
                self.back_input(command)?;
                Ok(ScannedLeaderPayload::Missing)
            }
        }
    }

    /// Scans TeX82's named glue-parameter assignment operand.
    ///
    /// This follows `scan_optional_equals` and `scan_glue`, retaining their
    /// canonical backup and alignment-delivery transitions before replay
    /// applies the aggregate mutation.
    pub fn scan_glue_parameter_assignment(
        &mut self,
        index: u16,
        mu: bool,
    ) -> Result<ScannedGlueParameterAssignment, CommandError> {
        let _ = self.scan_optional_equals()?;
        let value = self.scan_glue(mu)?.value;
        Ok(ScannedGlueParameterAssignment { index, value, mu })
    }

    /// Scans the complete expanded specification of a TeX82 rule.
    ///
    /// This is TeX.web's `scan_rule_spec`: keyword recognition and dimension
    /// scanning stay in command control, including failed-keyword replay.
    pub fn scan_rule_spec(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ScannedRuleSpec, CommandError> {
        let default_rule = Scaled::from_raw(26_214);
        let (mut width, mut height, mut depth) = if primitive == UnexpandablePrimitive::VRule {
            (Some(default_rule), None, None)
        } else {
            (None, Some(default_rule), Some(Scaled::from_raw(0)))
        };
        loop {
            if self.scan_keyword("width")?.value {
                width = Some(self.scan_dimension()?.value);
            } else if self.scan_keyword("height")?.value {
                height = Some(self.scan_dimension()?.value);
            } else if self.scan_keyword("depth")?.value {
                depth = Some(self.scan_dimension()?.value);
            } else {
                break;
            }
        }
        Ok(ScannedRuleSpec {
            width,
            height,
            depth,
        })
    }

    /// Validates a box body's required opening brace, then restores it for
    /// executor-owned group entry.
    pub fn scan_box_group_opening(&mut self) -> Result<bool, CommandError> {
        match self.scan_left_brace(true) {
            Ok(opening) => {
                self.back_input(opening)?;
                Ok(true)
            }
            // `scan_left_brace` has retained the rejected command through
            // its canonical backup. Replay opens the required group as the
            // inserted brace, then consumes that command as box material.
            Err(CommandError::InputInvariant) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Scans TeX82 §§1070--1071's complete box-construction prefix.
    pub fn scan_box_construction(
        &mut self,
        primitive: UnexpandablePrimitive,
    ) -> Result<ScannedBoxConstruction, CommandError> {
        let kind = match primitive {
            UnexpandablePrimitive::HBox => ScannedBoxKind::HBox,
            UnexpandablePrimitive::VBox => ScannedBoxKind::VBox,
            UnexpandablePrimitive::VTop => ScannedBoxKind::VTop,
            _ => return Err(CommandError::InputInvariant),
        };
        let packing = if self.scan_keyword("to")?.value {
            ScannedPackingSpec::Exactly(self.scan_dimension()?.value)
        } else if self.scan_keyword("spread")?.value {
            ScannedPackingSpec::Spread(self.scan_dimension()?.value)
        } else {
            ScannedPackingSpec::Natural
        };
        let opening_brace_replay = self.scan_box_group_opening()?;
        Ok(ScannedBoxConstruction {
            kind,
            packing,
            opening_brace_replay,
        })
    }

    /// Validates an alignment preamble's required opening brace, then restores
    /// it for canonical preamble delivery.
    ///
    /// TeX.web's `init_align` consumes this brace through expanded command
    /// input before `get_preamble_token` starts.  The subsequent
    /// `back_input` is therefore not an executor replay path: it is the one
    /// command-owned backup level whose literal-brace adjustment is undone
    /// before the preamble sees the same brace again.
    pub fn scan_alignment_preamble_opening(&mut self) -> Result<(), CommandError> {
        let opening = self.scan_left_brace(true)?;
        self.back_input(opening)
    }

    /// Replays the alignment preamble's opening brace into the next scanner
    /// episode.
    ///
    /// TeX.web's first `get_preamble_token` consumes the backup made by
    /// `init_align`, then backs that same brace up once more before the
    /// preamble scanner starts.  Retiring the exhausted first backup is part
    /// of this input transition, so it must remain command-owned rather than
    /// being recreated by replay main control.
    pub fn replay_alignment_preamble_opening(&mut self) -> Result<(), CommandError> {
        let opening = self.scan_left_brace(true)?;
        self.back_input_after_backup_replay(opening)
    }

    /// Delivers the first alignment cell's lookahead, then backs it up before
    /// the selected u-template is installed.
    ///
    /// This is TeX82's `init_col` lookahead ordering: every non-`\omit`
    /// command, including an ordinary unbraced cell such as `\vrule`, changes
    /// and then restores `align_state` through command-owned backup. `\omit`
    /// instead remains consumed and selects the typed template-free path.
    /// TeX82 §765 does not require the backed-up lookahead to be a left brace.
    pub fn scan_alignment_cell_opening(&mut self) -> Result<AlignmentCellOpening, CommandError> {
        loop {
            let opening = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
            match opening.meaning() {
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } => continue,
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit) => {
                    self.command
                        .prepare_alignment_cell_lookahead()
                        .map_err(|_| CommandError::InputInvariant)?;
                    return Ok(AlignmentCellOpening::Omit);
                }
                _ => {
                    self.back_input(opening)?;
                    return Ok(AlignmentCellOpening::Template);
                }
            }
        }
    }

    /// Performs TeX82 `fin_col`'s next-entry lookahead. Spaces are delivered
    /// normally; the first non-space token is restored before the selected
    /// u-template is installed.
    pub fn scan_alignment_next_cell_opening(
        &mut self,
    ) -> Result<AlignmentCellOpening, CommandError> {
        self.command
            .prepare_alignment_cell_lookahead()
            .map_err(|_| CommandError::InputInvariant)?;
        loop {
            let command = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
            if matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                continue;
            }
            if matches!(
                command.meaning(),
                Meaning::UnexpandablePrimitive(UnexpandablePrimitive::Omit)
            ) {
                return Ok(AlignmentCellOpening::Omit);
            }
            self.back_input(command)?;
            return Ok(AlignmentCellOpening::Template);
        }
    }

    /// Consumes the compulsory opener following `align_peek`'s `\\noalign`.
    ///
    /// TeX82 §37 sets `align_state := 1000000`, recognizes the expanded
    /// `no_align` command, then calls `scan_left_brace` before the executor
    /// creates `no_align_group`.  Unlike an `init_col` lookahead, this brace
    /// is not backed up: its raw delivery is the canonical `1000000 ->
    /// 1000001` transition.
    pub fn scan_alignment_noalign_opening(&mut self) -> Result<(), CommandError> {
        let _ = self.scan_left_brace(true)?;
        Ok(())
    }

    /// Installs TeX82 §37's `align_peek` sentinel before its expanded
    /// lookahead.  The command processor owns this state because it is raw
    /// token-delivery state, not executor group state.
    pub fn begin_alignment_peek(&mut self, after_noalign: bool) -> Result<(), CommandError> {
        let changed = self.command.alignment.align_state != 1_000_000;
        self.command
            .prepare_alignment_cell_lookahead()
            .map_err(|_| CommandError::InputInvariant)?;
        // TeX82 §37 assigns the `align_peek` sentinel before its first
        // expanded lookahead.  Keep that transition command-owned and emit
        // it before an exhausted backup is retired by `get_x_token`.
        #[cfg(any(test, feature = "instrumentation"))]
        if changed || after_noalign {
            self.observe(crate::CommandObservation::Alignment(
                crate::AlignmentRecord {
                    transition: "state_change",
                    alignment: self
                        .command
                        .alignment
                        .active_alignment
                        .map(|alignment| alignment.raw()),
                    align_state: self.command.alignment.align_state,
                    delimiter: None,
                    previous_align_state: None,
                },
            ));
        }
        Ok(())
    }

    /// Enters TeX82's live alignment-preamble scanner episode.
    ///
    /// `init_align` establishes `scanner_status := aligning` after its
    /// required brace has been replayed and backed up, but before the first
    /// `get_preamble_token` retires that backup.  The status therefore belongs
    /// to the command-owned input transition, rather than to executor replay
    /// or the preamble parser.
    pub fn begin_alignment_preamble_scan(&mut self) -> Result<(), CommandError> {
        // The second opener backup is retired by this expanded brace scan.
        // TeX82's following `get_preamble_token` must therefore start with
        // the first template token; an additional raw fetch would discard an
        // immediate `#` in a nested `\\halign{#\\cr}` preamble.
        let _ = self.scan_left_brace(true)?;
        let alignment = self
            .command
            .alignment
            .active_alignment
            .ok_or(CommandError::InputInvariant)?;
        self.command
            .alignment
            .set_preamble_phase(alignment)
            .map_err(|_| CommandError::InputInvariant)?;
        let _prior =
            self.command
                .begin_scanner_status(ScannerStatus::Aligning(AlignmentScanContext {
                    alignment: AlignmentId(alignment.raw()),
                    builder: TokenBuilderId(0),
                    warning: ScannerWarning(0),
                }));
        self.observe_scanner_status_transition(
            _prior.status().clone(),
            self.command.scanner.status().clone(),
        );
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(crate::CommandObservation::Alignment(
            crate::AlignmentRecord {
                transition: "preamble_start",
                alignment: Some(alignment.raw()),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            },
        ));
        let mut columns = Vec::new();
        let mut repeat_start = None;
        loop {
            // These are deliberately separate loops, matching TeX82 §760's
            // `done1`/`done2` labels. A missing `#` leaves the delivered
            // delimiter backed up, then the v-template loop reads it again.
            // A single combined u/v phase loses that replay boundary.
            let mut u_template = Vec::new();
            loop {
                let command = self.get_next()?.ok_or(CommandError::InputInvariant)?;
                let token = command.spelling().semantic_token();
                if matches!(
                    token,
                    Token::Char {
                        cat: Catcode::Parameter,
                        ..
                    }
                ) {
                    break;
                }
                let tab = matches!(
                    token,
                    Token::Char {
                        cat: Catcode::AlignmentTab,
                        ..
                    }
                );
                let terminator = tab
                    || matches!(
                        command.meaning(),
                        Meaning::UnexpandablePrimitive(
                            UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr
                        )
                    );
                if terminator && self.command.alignment.align_state == -1_000_000 {
                    // The `&&` case is the one exception: the second tab
                    // starts the periodic suffix and u-template scanning
                    // continues. Every other delimiter is TeX's
                    // `Missing # inserted` / `back_error` path.
                    if tab && u_template.is_empty() && repeat_start.is_none() {
                        repeat_start = Some(columns.len());
                        continue;
                    }
                    #[cfg(any(test, feature = "instrumentation"))]
                    self.observe(crate::CommandObservation::Alignment(
                        crate::AlignmentRecord {
                            transition: "missing_parameter",
                            alignment: Some(alignment.raw()),
                            align_state: self.command.alignment.align_state,
                            delimiter: None,
                            previous_align_state: None,
                        },
                    ));
                    self.back_error(command, MISSING_PARAMETER_DIAGNOSTIC)?;
                    break;
                }
                if !matches!(
                    command.meaning(),
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    }
                ) || !u_template.is_empty()
                {
                    // TeX82 §760 eliminates only leading u-template spaces.
                    u_template.push(command.spelling());
                }
            }

            let mut v_template = Vec::new();
            let ends_preamble = loop {
                let command = self.get_next()?.ok_or(CommandError::InputInvariant)?;
                let token = command.spelling().semantic_token();
                let ends_column = matches!(
                    token,
                    Token::Char {
                        cat: Catcode::AlignmentTab,
                        ..
                    }
                );
                let ends_preamble = matches!(
                    command.meaning(),
                    Meaning::UnexpandablePrimitive(
                        UnexpandablePrimitive::Cr | UnexpandablePrimitive::CrCr
                    )
                );
                if (ends_column || ends_preamble)
                    && self.command.alignment.align_state == -1_000_000
                {
                    break ends_preamble;
                }
                // §760 reports and discards extra parameter markers in a
                // v-template; it then resumes this same loop.
                if matches!(
                    token,
                    Token::Char {
                        cat: Catcode::Parameter,
                        ..
                    }
                ) {
                    #[cfg(any(test, feature = "instrumentation"))]
                    self.observe(crate::CommandObservation::Alignment(
                        crate::AlignmentRecord {
                            transition: "extra_parameter",
                            alignment: Some(alignment.raw()),
                            align_state: self.command.alignment.align_state,
                            delimiter: None,
                            previous_align_state: None,
                        },
                    ));
                    self.command
                        .expansion
                        .pending_diagnostics
                        .push(EXTRA_PARAMETER_DIAGNOSTIC);
                    continue;
                }
                v_template.push(command.spelling());
            };
            columns.push(AlignmentCellTemplates {
                // `init_col` installs a u-template even when its token list
                // is empty. `None` is reserved for the typed `\\omit` path.
                u_template: Some(self.state.finish_traced_token_list(&u_template)),
                v_template: self.state.finish_traced_token_list(&v_template),
            });
            if ends_preamble {
                break;
            }
        }
        self.command
            .alignment
            .complete_preamble(
                alignment,
                AlignmentPreamble {
                    columns,
                    repeat_start,
                },
            )
            .map_err(|_| CommandError::InputInvariant)?;
        #[cfg(any(test, feature = "instrumentation"))]
        self.observe(crate::CommandObservation::Alignment(
            crate::AlignmentRecord {
                transition: "preamble_finish",
                alignment: Some(alignment.raw()),
                align_state: self.command.alignment.align_state,
                delimiter: None,
                previous_align_state: None,
            },
        ));
        // TeX's `fin_align` boundary becomes observable before `scanner_status`
        // returns to normal. Retain the live aligning episode while publishing
        // its completion, then restore normal status; otherwise an exit record
        // loses its `aligning` identity and reverses the canonical ordering.
        let prior = self.command.begin_scanner_status(ScannerStatus::Normal);
        self.observe_scanner_status_transition(
            prior.status().clone(),
            self.command.scanner.status().clone(),
        );
        Ok(())
    }

    /// Scans TeX's balanced general text through the canonical `scan_toks`
    /// collector. `expanded` controls its TeX82 expanded-collection mode.
    pub fn scan_balanced_text(
        &mut self,
        expanded: bool,
    ) -> Result<ScannedBalancedText, CommandError> {
        // Expanded general text (for `\message`, for example) enters its
        // absorbing episode before the required opening brace. The ordinary
        // token-list assignment scan retains TeX82's initial validation and
        // replay, whose backup transition is separately observable.
        if !expanded {
            let opening = self.scan_left_brace(true)?;
            self.back_input(opening)?;
        }
        let scanned = self.scan_toks(ScanToksMode::General { expanded })?;
        Ok(ScannedBalancedText {
            tokens: scanned.replacement_text,
            provenance: provenance(&scanned),
        })
    }

    /// Scans a macro parameter text and replacement text without exposing the
    /// temporary macro-argument matcher or its input frames.
    pub fn scan_macro_definition(
        &mut self,
        expanded: bool,
    ) -> Result<ScannedMacroDefinition, CommandError> {
        let command = self
            .next_non_space_raw()?
            .ok_or(CommandError::InputInvariant)?;
        let (target, missing_target) = if let Some(target) = command.control_sequence() {
            (target, false)
        } else {
            self.back_input(command)?;
            (self.state.intern_control_sequence("inaccessible"), true)
        };
        let scanned = self.scan_toks(ScanToksMode::MacroDefinition { expanded })?;
        Ok(ScannedMacroDefinition {
            target,
            parameter_text: scanned.parameter_text,
            replacement_text: scanned.replacement_text,
            provenance: provenance(&scanned),
            missing_target,
            malformed_parameter: scanned.malformed_parameter,
        })
    }

    /// Scans TeX82's raw `\let` operand sequence.
    ///
    /// `future` selects `future_let`: the first two raw tokens following the
    /// target are restored in their original order after the second token's
    /// meaning has been captured.
    pub fn scan_let_assignment(
        &mut self,
        future: bool,
    ) -> Result<ScannedLetAssignment, CommandError> {
        let target = self
            .next_non_space_raw()?
            .and_then(|command| command.control_sequence())
            .ok_or(CommandError::InputInvariant)?;
        let (source, meaning) = if future {
            let first = self.get_token()?.ok_or(CommandError::InputInvariant)?;
            let second = self.get_token()?.ok_or(CommandError::InputInvariant)?;
            let source = second.control_sequence();
            let meaning = second.meaning();
            self.replay_raw_commands([first, second]);
            (source, meaning)
        } else {
            let mut source = self.get_token()?.ok_or(CommandError::InputInvariant)?;
            if matches!(source.meaning(), Meaning::CharToken { ch: '=', .. }) {
                source = self.get_token()?.ok_or(CommandError::InputInvariant)?;
                if matches!(
                    source.meaning(),
                    Meaning::CharToken {
                        cat: Catcode::Space,
                        ..
                    }
                ) {
                    source = self.get_token()?.ok_or(CommandError::InputInvariant)?;
                }
            }
            (source.control_sequence(), source.meaning())
        };
        Ok(ScannedLetAssignment {
            target,
            source,
            meaning,
        })
    }

    /// TeX's `scan_file_name`, returning a typed boundary instead of an input
    /// cursor or a backed-up raw command.
    pub fn scan_file_name(&mut self) -> Result<ScannedFileName, CommandError> {
        let first = loop {
            let command = self.get_x_token()?.ok_or(CommandError::InputInvariant)?;
            if !matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                break command;
            }
        };
        let provenance = StructuredProvenance {
            primary: first.origin(),
        };
        let grouped = matches!(
            first.meaning(),
            Meaning::CharToken {
                cat: Catcode::BeginGroup,
                ..
            }
        );
        // `scan_file_name` replays its first non-space token before consuming
        // the filename. TeX82 exposes this `back_input` hand-off, and it
        // keeps the group-opening case on the same ordinary delivery path.
        self.back_input(first)?;
        let mut name = String::new();
        let mut quoted = false;
        let mut next = None;
        let termination = loop {
            let command = match next.take() {
                Some(command) => command,
                None => match self.get_x_token()? {
                    Some(command) => command,
                    None => break FileNameTermination::EndOfInput,
                },
            };
            match command.meaning() {
                Meaning::CharToken {
                    cat: Catcode::BeginGroup,
                    ..
                } if grouped => {}
                Meaning::CharToken { ch: '"', .. } => quoted = !quoted,
                Meaning::CharToken {
                    cat: Catcode::EndGroup,
                    ..
                } if grouped && !quoted => {
                    break FileNameTermination::Group;
                }
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                } if !grouped && !quoted => {
                    break FileNameTermination::Space;
                }
                Meaning::CharToken { ch, .. } => name.push(ch),
                _ if !grouped => {
                    self.back_input(command)?;
                    break FileNameTermination::NonCharacter;
                }
                _ => return Err(CommandError::InputInvariant),
            }
        };
        if name.is_empty() {
            return Err(CommandError::InputInvariant);
        }
        Ok(ScannedFileName {
            name,
            termination,
            provenance,
        })
    }

    /// Scans and opens one input through the borrow-scoped registered-input
    /// capability. No filesystem or host lookup escapes this boundary.
    pub fn open_registered_input(&mut self) -> Result<RegisteredInput, CommandError> {
        let file_name = self.scan_file_name()?;
        let source = self
            .host
            .input(&file_name.name)
            .ok_or(CommandError::MissingInput)?;
        let source = self
            .command
            .register_source(source)
            .map_err(|_| CommandError::InputInvariant)?;
        self.command
            .open_registered_source(source)
            .map_err(|_| CommandError::InputInvariant)?;
        Ok(RegisteredInput { file_name, source })
    }

    fn next_non_space_raw(&mut self) -> Result<Option<crate::CurrentCommand>, CommandError> {
        loop {
            let Some(command) = self.get_token()? else {
                return Ok(None);
            };
            if !matches!(
                command.meaning(),
                Meaning::CharToken {
                    cat: Catcode::Space,
                    ..
                }
            ) {
                return Ok(Some(command));
            }
        }
    }

    fn replay_raw_commands(&mut self, commands: [crate::CurrentCommand; 2]) {
        for command in &commands {
            self.undo_alignment_delivery(command);
        }
        self.command.push_token_level(
            crate::input::TokenPayload::BackedUp(crate::input::SharedBackedUpBuffer::new(
                commands.map(|command| crate::input::BackedUpToken {
                    spelling: command.spelling(),
                    source_provenance: command.source_provenance(),
                }),
            )),
            crate::input::TokenBehavior::BackedUp(crate::input::BackupTreatment::Ordinary),
            crate::input::RetirementBehavior::Pop,
            crate::input::ReplayTrace::BackedUp,
        );
    }
}

fn provenance(scanned: &ScannedToks) -> StructuredProvenance {
    StructuredProvenance {
        primary: scanned.primary,
    }
}

#[cfg(test)]
mod tests;
