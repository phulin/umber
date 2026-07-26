//! Exhaustive TeX82/e-TeX/pdfTeX command-family classification.
//!
//! `canonical_command_identity` (in the parent module) used to fall back to a
//! generic `"unexpandable"`/`"expandable"` identity for any
//! `UnexpandablePrimitive`/`ExpandablePrimitive` variant it did not name
//! explicitly. That silent wildcard meant a primitive newly given its own
//! variant -- or simply never gotten to -- produced an indistinguishable
//! generic trace identity instead of a build failure, so the gap surfaced
//! only when some fixture happened to exercise it (`umber2-johp.65`,
//! `umber2-c8ul`).
//!
//! The two functions below are each an EXHAUSTIVE match over their primitive
//! enum: adding a variant to `tex_state::meaning::UnexpandablePrimitive` or
//! `ExpandablePrimitive` without extending the matching arm here is a build
//! failure (`error[E0004]: non-exhaustive patterns`), not a silent generic
//! fallback. This follows the same dispatch-completeness precedent
//! `umber2-johp.69` established for `scan_command`
//! (`docs/tex_command_core.md` §33.2), applied here to the classifier
//! instead of to execution dispatch.
//!
//! Ground truth for every classic TeX82 identity below was taken directly
//! from `third_party/reference/tex/tex/tex.web`'s `primitive(...)` calls
//! (cross-referenced against the exact Rust primitive spelling registered in
//! `crates/tex-exec/src/assignments/primitives.rs`), or, for the handful of
//! `def_code`/`def_family`/`make_box`/`set_box_dimen` identities whose
//! selector is a real eqtb address or mode-shifted constant rather than a
//! small literal, from a live run of the pinned instrumented TeX82 oracle
//! (`scripts/build-tex82-oracle.sh`) built from the same pinned TeX Live 2025
//! source this repository's committed fixtures are generated against. Those
//! probed values are cited by their probe below rather than by section,
//! since they are build-specific eqtb addresses rather than portable
//! tex.web constants.
//!
//! e-TeX identities cite `/tmp/etex.ch` (the pinned e-TeX 2.6 change file
//! used to build `tests/etex26-oracle`); their arithmetic is self-contained
//! within that change file's own symbolic constants and is not sensitive to
//! the build-specific eqtb layout noted above.
//!
//! pdfTeX-only primitives (the `Pdf*` variants, plus a handful of
//! engine-neutral Umber/LaTeX-support additions) are not exercised by any
//! committed fixture in this repository today -- the only wired fixture
//! registry (`tests/corpus/command/tex82`) replays the plain TeX82 INITEX
//! dialect exclusively. Their identities below are BEST-EFFORT: chosen to be
//! architecturally plausible (pdfTeX is not known to add any top-level
//! command code beyond TeX82/e-TeX's 117; see `tests/pdftex14027-oracle/
//! instrumentation.ch`'s `umber_trace_command_name`, which is byte-identical
//! to TeX82's and e-TeX's) and internally distinguishable, but not verified
//! against a live pdfTeX reference. Expect some of them to be the exact
//! divergence a future pdfTeX-dialect fixture reports; that is the intended,
//! accepted outcome of exhaustiveness over precision for primitives no
//! committed oracle exercises yet, not a defect in this change.

use tex_state::meaning::{ExpandablePrimitive, UnexpandablePrimitive};

/// TeX82 `@d vmode=1` (`\prevdepth`/`\nointerlineskip`'s `set_aux` selector).
const VMODE: i64 = 1;
/// TeX82 `@d hmode=vmode+max_command+1`, `max_command=100`
/// (`\spacefactor`'s `set_aux` selector).
const HMODE: i64 = VMODE + 100 + 1;
/// TeX82 `@d width_offset=1`.
const WIDTH_OFFSET: i64 = 1;
/// TeX82 `@d height_offset=3`.
const HEIGHT_OFFSET: i64 = 3;
/// TeX82 `@d depth_offset=2`.
const DEPTH_OFFSET: i64 = 2;

/// Canonical TeX82/e-TeX/pdfTeX command identity for a delivered
/// `UnexpandablePrimitive`. See the module documentation for the ground
/// truth each arm is based on.
pub(crate) fn unexpandable_primitive_identity(
    primitive: UnexpandablePrimitive,
) -> (String, Option<i64>) {
    use UnexpandablePrimitive as P;
    match primitive {
        P::Def => ("def".into(), Some(0)),
        P::Gdef => ("def".into(), Some(1)),
        P::Edef => ("def".into(), Some(2)),
        P::Xdef => ("def".into(), Some(3)),
        // `\globaldefs` is installed as `Meaning::IntParam` (tex.web
        // `assign_int`), never constructed as this primitive variant; kept
        // here only so the match stays exhaustive if that ever changes.
        P::GlobalDefs => ("assign_int".into(), None),
        P::Long => ("prefix".into(), Some(1)),
        P::Outer => ("prefix".into(), Some(2)),
        P::Global => ("prefix".into(), Some(4)),
        // e-TeX (`/tmp/etex.ch`): `primitive("protected",prefix,8)`.
        P::Protected => ("prefix".into(), Some(8)),
        P::Let => ("let".into(), Some(0)),
        P::FutureLet => ("let".into(), Some(1)),
        // tex.web: `register` shares `int_val`/`dimen_val`/`glue_val`/`mu_val`
        // (0/1/2/3) as its selector for `\count`/`\dimen`/`\skip`/`\muskip`.
        P::Count => ("register".into(), Some(0)),
        P::Dimen => ("register".into(), Some(1)),
        P::Skip => ("register".into(), Some(2)),
        P::Muskip => ("register".into(), Some(3)),
        P::Toks => ("toks_register".into(), Some(0)),
        // tex.web `shorthand_def`: char_def_code=0, math_char_def_code=1,
        // count_def_code=2, dimen_def_code=3, skip_def_code=4,
        // mu_skip_def_code=5, toks_def_code=6.
        P::CharDef => ("shorthand_def".into(), Some(0)),
        P::MathCharDef => ("shorthand_def".into(), Some(1)),
        P::CountDef => ("shorthand_def".into(), Some(2)),
        P::DimenDef => ("shorthand_def".into(), Some(3)),
        P::SkipDef => ("shorthand_def".into(), Some(4)),
        P::MuskipDef => ("shorthand_def".into(), Some(5)),
        P::ToksDef => ("shorthand_def".into(), Some(6)),
        P::Advance => ("advance".into(), Some(0)),
        P::Multiply => ("multiply".into(), Some(0)),
        P::Divide => ("divide".into(), Some(0)),
        // `def_code`'s selector is a real eqtb address (tex.web's
        // `cat_code_base`/`lc_code_base`/`uc_code_base`/`sf_code_base`/
        // `math_code_base`/`del_code_base` chain), not a small literal.
        // CatCode/LcCode/UcCode were previously established
        // (`umber2-johp.65`); SfCode/MathCode/DelCode were confirmed by a
        // live probe of the pinned TeX82 oracle (`\sfcode`/`\mathcode`/
        // `\delcode` on a fresh INITEX, 2026-07-26).
        P::CatCode => ("def_code".into(), Some(25_631)),
        P::LcCode => ("def_code".into(), Some(25_887)),
        P::UcCode => ("def_code".into(), Some(26_143)),
        P::SfCode => ("def_code".into(), Some(26_399)),
        P::MathCode => ("def_code".into(), Some(26_655)),
        P::DelCode => ("def_code".into(), Some(27_485)),
        P::Font => ("def_font".into(), Some(0)),
        P::FontDimen => ("assign_font_dimen".into(), Some(0)),
        P::HyphenChar => ("assign_font_int".into(), Some(0)),
        P::SkewChar => ("assign_font_int".into(), Some(1)),
        P::Hyphenation => ("hyph_data".into(), Some(0)),
        P::Patterns => ("hyph_data".into(), Some(1)),
        P::Par => ("par_end".into(), Some(256)),
        // `\endgraf` is Umber's own primitive alias for `\par` (plain.tex
        // ordinarily provides it via `\let`); every dispatch site treats the
        // two identically, so they share `par_end`'s identity too.
        P::EndGraf => ("par_end".into(), Some(256)),
        P::Indent => ("start_par".into(), Some(1)),
        P::NoIndent => ("start_par".into(), Some(0)),
        P::ParShape => ("set_shape".into(), Some(0)),
        P::PrevDepth => ("set_aux".into(), Some(VMODE)),
        // `\nointerlineskip` is Umber's own fixed-chr-code primitive for what
        // plain.tex ordinarily spells `\prevdepth=-1000pt` (`umber2-johp.69`);
        // it performs the identical `set_aux` assignment as `\prevdepth`.
        P::NoInterlineSkip => ("set_aux".into(), Some(VMODE)),
        P::PrevGraf => ("set_prev_graf".into(), Some(0)),
        P::HAlign => ("halign".into(), Some(0)),
        P::VAlign => ("valign".into(), Some(0)),
        P::NoAlign => ("no_align".into(), Some(0)),
        P::Omit => ("omit".into(), Some(0)),
        P::Cr => ("car_ret".into(), Some(257)),
        P::CrCr => ("car_ret".into(), Some(258)),
        // tex.web: `primitive("span",tab_mark,span_code)`, span_code=256.
        P::Span => ("tab_mark".into(), Some(256)),
        // `make_box`'s selector distinguishes each box constructor. `\box`,
        // `\copy`, `\lastbox`, `\vsplit`, `\vtop`, `\vbox` were confirmed by a
        // live oracle probe (2026-07-26); `\hbox`'s chr is `vtop_code+hmode`
        // (tex.web `@d hmode=vmode+max_command+1`, max_command=100), which
        // the probe also confirmed as 106 -- correcting a previously wrong
        // value (4) that was never exercised by the committed fixture.
        P::Box => ("make_box".into(), Some(0)),
        P::Copy => ("make_box".into(), Some(1)),
        P::LastBox => ("make_box".into(), Some(2)),
        P::VSplit => ("make_box".into(), Some(3)),
        P::VTop => ("make_box".into(), Some(4)),
        P::VBox => ("make_box".into(), Some(5)),
        P::HBox => ("make_box".into(), Some(106)),
        P::SetBox => ("set_box".into(), Some(0)),
        // `un_hbox`/`un_vbox` share `box_code`(0)/`copy_code`(1); confirmed
        // by the same live probe.
        P::UnHBox => ("un_hbox".into(), Some(0)),
        P::UnHCopy => ("un_hbox".into(), Some(1)),
        P::UnVBox => ("un_vbox".into(), Some(0)),
        P::UnVCopy => ("un_vbox".into(), Some(1)),
        P::Wd => ("set_box_dimen".into(), Some(WIDTH_OFFSET)),
        P::Ht => ("set_box_dimen".into(), Some(HEIGHT_OFFSET)),
        P::Dp => ("set_box_dimen".into(), Some(DEPTH_OFFSET)),
        // tex.web: `primitive("moveleft",hmove,1)`, `("moveright",hmove,0)`,
        // `("raise",vmove,1)`, `("lower",vmove,0)`.
        P::MoveLeft => ("hmove".into(), Some(1)),
        P::MoveRight => ("hmove".into(), Some(0)),
        P::Raise => ("vmove".into(), Some(1)),
        P::Lower => ("vmove".into(), Some(0)),
        P::Char => ("char_num".into(), Some(0)),
        // tex.web: `primitive("kern",kern,explicit)`, explicit=1.
        P::Kern => ("kern".into(), Some(1)),
        P::HSkip => ("hskip".into(), Some(4)),
        P::VSkip => ("vskip".into(), Some(4)),
        P::HFil => ("hskip".into(), Some(0)),
        P::HFill => ("hskip".into(), Some(1)),
        P::HSs => ("hskip".into(), Some(2)),
        P::HFilNeg => ("hskip".into(), Some(3)),
        P::VFil => ("vskip".into(), Some(0)),
        P::VFill => ("vskip".into(), Some(1)),
        P::VSs => ("vskip".into(), Some(2)),
        P::VFilNeg => ("vskip".into(), Some(3)),
        // tex.web `leader_ship`: a_leaders=100 (shipout uses a_leaders-1=99),
        // c_leaders=101, x_leaders=102.
        P::Shipout => ("leader_ship".into(), Some(99)),
        P::Leaders => ("leader_ship".into(), Some(100)),
        P::CLeaders => ("leader_ship".into(), Some(101)),
        P::XLeaders => ("leader_ship".into(), Some(102)),
        P::Penalty => ("break_penalty".into(), Some(0)),
        P::VRule => ("vrule".into(), Some(0)),
        P::HRule => ("hrule".into(), Some(0)),
        P::ControlSpace => ("ex_space".into(), Some(0)),
        P::ItalicCorrection => ("ital_corr".into(), Some(0)),
        P::Discretionary => ("discretionary".into(), Some(0)),
        P::DiscretionaryHyphen => ("discretionary".into(), Some(1)),
        P::NoBoundary => ("no_boundary".into(), Some(0)),
        P::SpaceFactor => ("set_aux".into(), Some(HMODE)),
        P::Accent => ("accent".into(), Some(0)),
        P::Mark => ("mark".into(), Some(0)),
        // e-TeX: `primitive("marks",mark,marks_code)`, marks_code=5.
        P::Marks => ("mark".into(), Some(5)),
        P::VAdjust => ("vadjust".into(), Some(0)),
        P::Insert => ("insert".into(), Some(0)),
        // `remove_item`'s selector is the node type it removes: glue_node=10,
        // kern_node=11, penalty_node=12 (confirmed by live probe).
        P::UnSkip => ("remove_item".into(), Some(10)),
        P::UnKern => ("remove_item".into(), Some(11)),
        P::UnPenalty => ("remove_item".into(), Some(12)),
        // `last_item` selectors int_val=0/dimen_val=1/glue_val=2.
        P::LastPenalty => ("last_item".into(), Some(0)),
        P::LastKern => ("last_item".into(), Some(1)),
        P::LastSkip => ("last_item".into(), Some(2)),
        P::OpenIn => ("in_stream".into(), Some(1)),
        P::CloseIn => ("in_stream".into(), Some(0)),
        P::OpenOut => ("extension".into(), Some(0)),
        P::Write => ("extension".into(), Some(1)),
        P::CloseOut => ("extension".into(), Some(2)),
        P::Special => ("extension".into(), Some(3)),
        P::Immediate => ("extension".into(), Some(4)),
        P::SetLanguage => ("extension".into(), Some(5)),
        P::Read => ("read_to_cs".into(), Some(0)),
        // e-TeX: `primitive("readline",read_to_cs,1)`.
        P::ReadLine => ("read_to_cs".into(), Some(1)),
        // e-TeX `last_item` extension block, all self-consistent within
        // `/tmp/etex.ch`'s own symbols (badness_code=5, eTeX_int=6,
        // eTeX_dim=14, eTeX_glue=23, eTeX_mu=24, eTeX_expr=25 under e-TeX's
        // redefinition of `input_line_no_code`/`badness_code`).
        P::FontCharWd => ("last_item".into(), Some(14)),
        P::FontCharHt => ("last_item".into(), Some(15)),
        P::FontCharDp => ("last_item".into(), Some(16)),
        P::FontCharIc => ("last_item".into(), Some(17)),
        P::ParShapeLength => ("last_item".into(), Some(18)),
        P::ParShapeIndent => ("last_item".into(), Some(19)),
        P::ParShapeDimen => ("last_item".into(), Some(20)),
        P::GlueStretch => ("last_item".into(), Some(21)),
        P::GlueShrink => ("last_item".into(), Some(22)),
        P::GlueStretchOrder => ("last_item".into(), Some(12)),
        P::GlueShrinkOrder => ("last_item".into(), Some(13)),
        P::MuToGlue => ("last_item".into(), Some(23)),
        P::GlueToMu => ("last_item".into(), Some(24)),
        P::NumExpr => ("last_item".into(), Some(25)),
        P::DimExpr => ("last_item".into(), Some(26)),
        P::GlueExpr => ("last_item".into(), Some(27)),
        P::MuExpr => ("last_item".into(), Some(28)),
        // e-TeX `set_shape` penalty-list extension: BEST-EFFORT ordinal, not
        // the real `etex_pen_base`-relative eqtb address (that base chains
        // through `int_base`, which this build's confirmed value shows is
        // not the vanilla tex.web offset -- see the module documentation).
        P::InterLinePenalties => ("set_shape".into(), Some(1)),
        P::ClubPenalties => ("set_shape".into(), Some(2)),
        P::WidowPenalties => ("set_shape".into(), Some(3)),
        P::DisplayWidowPenalties => ("set_shape".into(), Some(4)),
        // e-TeX: `primitive("pagediscards",un_vbox,last_box_code)`,
        // `primitive("splitdiscards",un_vbox,vsplit_code)`.
        P::PageDiscards => ("un_vbox".into(), Some(2)),
        P::SplitDiscards => ("un_vbox".into(), Some(3)),
        // e-TeX: `primitive("interactionmode",set_page_int,2)`.
        P::InteractionMode => ("set_page_int".into(), Some(2)),
        // e-TeX `xray` extensions: show_groups=4, show_tokens=5, show_ifs=6.
        P::ShowGroups => ("xray".into(), Some(4)),
        P::ShowTokens => ("xray".into(), Some(5)),
        P::ShowIfs => ("xray".into(), Some(6)),
        // e-TeX `valign`/`left_right` bidi extensions. `Middle`'s chr is
        // e-TeX's own `middle_noad=1` subtype constant (confirmed directly);
        // BeginL/EndL/BeginR/EndR derive from `/tmp/etex.ch`'s `L_code=4`,
        // `R_code=8`, `begin_M_code=2`, `end_M_code=3` assuming the vanilla
        // tex.web convention `before=0` (not itself re-stated in the change
        // file), so these four are BEST-EFFORT pending direct confirmation.
        P::BeginL => ("valign".into(), Some(6)),
        P::EndL => ("valign".into(), Some(7)),
        P::BeginR => ("valign".into(), Some(10)),
        P::EndR => ("valign".into(), Some(11)),
        P::Middle => ("left_right".into(), Some(1)),
        P::BeginGroup => ("begin_group".into(), Some(0)),
        P::EndGroup => ("end_group".into(), Some(0)),
        P::AfterGroup => ("after_group".into(), Some(0)),
        P::AfterAssignment => ("after_assignment".into(), Some(0)),
        // `xray`'s selector: show_code=0, show_box_code=1, show_the_code=2,
        // show_lists_code=3.
        P::Show => ("xray".into(), Some(0)),
        P::ShowBox => ("xray".into(), Some(1)),
        P::ShowThe => ("xray".into(), Some(2)),
        P::ShowLists => ("xray".into(), Some(3)),
        P::Message => ("message".into(), Some(0)),
        P::ErrMessage => ("message".into(), Some(1)),
        // Not a tex.web primitive; no e-TeX/pdfTeX registration was found
        // either. BEST-EFFORT: kept in the `xray` diagnostic family it
        // behaves like, with a selector past e-TeX's highest (`show_ifs`=6).
        P::ShowHyphens => ("xray".into(), Some(7)),
        // tex.web: `primitive("uppercase",case_shift,uc_code_base)`,
        // `primitive("lowercase",case_shift,lc_code_base)` -- the same
        // eqtb addresses already confirmed for `\uccode`/`\lccode`
        // (`umber2-c8ul`).
        P::Uppercase => ("case_shift".into(), Some(26_143)),
        P::Lowercase => ("case_shift".into(), Some(25_887)),
        P::IgnoreSpaces => ("ignore_spaces".into(), Some(0)),
        P::MathChar => ("math_char_num".into(), Some(0)),
        P::Delimiter => ("delim_num".into(), Some(0)),
        // `def_family`'s selector is `math_font_base` (+`script_size`=16,
        // +`script_script_size`=32), confirmed by the same live probe as the
        // `def_code` family above.
        P::TextFont => ("def_family".into(), Some(25_583)),
        P::ScriptFont => ("def_family".into(), Some(25_599)),
        P::ScriptScriptFont => ("def_family".into(), Some(25_615)),
        // `math_comp`'s selector is the noad type: unset_node=13, so
        // ord_noad=16 through inner_noad=23, then radical_noad=24,
        // fraction_noad=25, under_noad=26, over_noad=27.
        P::MathOrd => ("math_comp".into(), Some(16)),
        P::MathOp => ("math_comp".into(), Some(17)),
        P::MathBin => ("math_comp".into(), Some(18)),
        P::MathRel => ("math_comp".into(), Some(19)),
        P::MathOpen => ("math_comp".into(), Some(20)),
        P::MathClose => ("math_comp".into(), Some(21)),
        P::MathPunct => ("math_comp".into(), Some(22)),
        P::MathInner => ("math_comp".into(), Some(23)),
        P::Underline => ("math_comp".into(), Some(26)),
        P::Overline => ("math_comp".into(), Some(27)),
        // `limit_switch`: normal=0, limits=1, no_limits=2.
        P::DisplayLimits => ("limit_switch".into(), Some(0)),
        P::Limits => ("limit_switch".into(), Some(1)),
        P::NoLimits => ("limit_switch".into(), Some(2)),
        // `above`: above_code=0, over_code=1, atop_code=2,
        // delimited_code(3)+{above,over,atop}_code.
        P::Above => ("above".into(), Some(0)),
        P::Over => ("above".into(), Some(1)),
        P::Atop => ("above".into(), Some(2)),
        P::AboveWithDelims => ("above".into(), Some(3)),
        P::OverWithDelims => ("above".into(), Some(4)),
        P::AtopWithDelims => ("above".into(), Some(5)),
        P::Radical => ("radical".into(), Some(0)),
        P::MathAccent => ("math_accent".into(), Some(0)),
        P::VCenter => ("vcenter".into(), Some(0)),
        // tex.web: `primitive("mskip",mskip,mskip_code)`, mskip_code=5;
        // `primitive("mkern",mkern,mu_glue)`, mu_glue=99.
        P::MSkip => ("mskip".into(), Some(5)),
        P::MKern => ("mkern".into(), Some(99)),
        P::NonScript => ("non_script".into(), Some(0)),
        P::MathChoice => ("math_choice".into(), Some(0)),
        // `left_right`: left_noad=30, right_noad=31 (vcenter_noad+1/+2).
        P::Left => ("left_right".into(), Some(30)),
        P::Right => ("left_right".into(), Some(31)),
        P::EqNo => ("eq_no".into(), Some(0)),
        P::LeftEqNo => ("eq_no".into(), Some(1)),
        // `math_style`: display_style=0, text_style=2, script_style=4,
        // script_script_style=6.
        P::DisplayStyle => ("math_style".into(), Some(0)),
        P::TextStyle => ("math_style".into(), Some(2)),
        P::ScriptStyle => ("math_style".into(), Some(4)),
        P::ScriptScriptStyle => ("math_style".into(), Some(6)),
        // `set_interaction`: batch_mode=0, nonstop_mode=1, scroll_mode=2,
        // error_stop_mode=3.
        P::BatchMode => ("set_interaction".into(), Some(0)),
        P::NonstopMode => ("set_interaction".into(), Some(1)),
        P::ScrollMode => ("set_interaction".into(), Some(2)),
        P::ErrorStopMode => ("set_interaction".into(), Some(3)),
        P::End => ("stop".into(), Some(0)),
        P::Dump => ("stop".into(), Some(1)),
        // BEST-EFFORT pdfTeX and engine-neutral Umber/LaTeX-extension
        // identities below. None of these primitives are exercised by any
        // committed fixture in this repository (see the module
        // documentation): the wired registry replays TeX82 INITEX only.
        // pdfTeX is architecturally known not to add any top-level command
        // code beyond TeX82/e-TeX's 117 (`tests/pdftex14027-oracle/
        // instrumentation.ch`'s `umber_trace_command_name` table is
        // byte-identical to TeX82's and e-TeX's own), so every whatsit-like
        // pdfTeX primitive below reuses the `extension` family with a
        // sequential selector continuing after TeX82's `set_language_code`=5,
        // and per-character pdfTeX "code" tables reuse `assign_font_int`
        // continuing after `\skewchar`=1. These sequential selectors are
        // ordinal placeholders in Rust enum declaration order, not confirmed
        // pdfTeX chr values.
        P::PdfLpCode => ("assign_font_int".into(), Some(2)),
        P::PdfRpCode => ("assign_font_int".into(), Some(3)),
        P::PdfEfCode => ("assign_font_int".into(), Some(4)),
        P::PdfTagCode => ("assign_font_int".into(), Some(5)),
        P::PdfKnbsCode => ("assign_font_int".into(), Some(6)),
        P::PdfStbsCode => ("assign_font_int".into(), Some(7)),
        P::PdfShbsCode => ("assign_font_int".into(), Some(8)),
        P::PdfKnbcCode => ("assign_font_int".into(), Some(9)),
        P::PdfKnacCode => ("assign_font_int".into(), Some(10)),
        P::PdfNoLigatures => ("assign_font_int".into(), Some(11)),
        P::LetterspaceFont => ("extension".into(), Some(6)),
        P::PdfCopyFont => ("extension".into(), Some(7)),
        P::PdfFontExpand => ("extension".into(), Some(8)),
        P::PdfFontAttr => ("extension".into(), Some(9)),
        P::PdfIncludeChars => ("extension".into(), Some(10)),
        P::PdfMapFile => ("extension".into(), Some(11)),
        P::PdfMapLine => ("extension".into(), Some(12)),
        P::PdfGlyphToUnicode => ("extension".into(), Some(13)),
        P::PdfNoBuiltinToUnicode => ("extension".into(), Some(14)),
        P::PdfLiteral => ("extension".into(), Some(15)),
        P::PdfSetMatrix => ("extension".into(), Some(16)),
        P::PdfSave => ("extension".into(), Some(17)),
        P::PdfRestore => ("extension".into(), Some(18)),
        P::PdfColorStack => ("extension".into(), Some(19)),
        P::PdfSavePos => ("extension".into(), Some(20)),
        P::PdfSnapRefPoint => ("extension".into(), Some(21)),
        P::PdfSnapY => ("extension".into(), Some(22)),
        P::PdfSnapYComp => ("extension".into(), Some(23)),
        P::PdfXForm => ("extension".into(), Some(24)),
        P::PdfRefXForm => ("extension".into(), Some(25)),
        // Umber's own sentinel for a registered pdfTeX name not yet routed
        // to real behavior; deliberately distinct from every real identity.
        P::PdfTeXUnimplemented => ("pdftex_unimplemented".into(), None),
        P::PdfResetTimer => ("extension".into(), Some(26)),
        P::PdfSetRandomSeed => ("extension".into(), Some(27)),
        P::PdfObject => ("extension".into(), Some(28)),
        P::PdfReferenceObject => ("extension".into(), Some(29)),
        P::PdfInfo => ("extension".into(), Some(30)),
        P::PdfCatalog => ("extension".into(), Some(31)),
        P::PdfNames => ("extension".into(), Some(32)),
        P::PdfTrailer => ("extension".into(), Some(33)),
        P::PdfTrailerId => ("extension".into(), Some(34)),
        P::PdfInterwordSpaceOn => ("extension".into(), Some(35)),
        P::PdfInterwordSpaceOff => ("extension".into(), Some(36)),
        P::PdfFakeSpace => ("extension".into(), Some(37)),
        P::PdfSpaceFont => ("extension".into(), Some(38)),
        P::PdfAnnot => ("extension".into(), Some(39)),
        P::PdfStartLink => ("extension".into(), Some(40)),
        P::PdfEndLink => ("extension".into(), Some(41)),
        P::PdfRunningLinkOn => ("extension".into(), Some(42)),
        P::PdfRunningLinkOff => ("extension".into(), Some(43)),
        // Behaves like `\leavevmode`: starts an (empty) paragraph only in
        // vertical mode, without indentation -- the same family as
        // `\noindent`.
        P::QuitVMode => ("start_par".into(), Some(0)),
        P::PdfOutline => ("extension".into(), Some(44)),
        P::PdfDest => ("extension".into(), Some(45)),
        P::PdfThread => ("extension".into(), Some(46)),
        P::PdfStartThread => ("extension".into(), Some(47)),
        P::PdfEndThread => ("extension".into(), Some(48)),
        P::PdfXImage => ("extension".into(), Some(49)),
        P::PdfRefXImage => ("extension".into(), Some(50)),
    }
}

/// Canonical TeX82/e-TeX/pdfTeX command identity for a delivered
/// `ExpandablePrimitive`. See the module documentation for the ground truth
/// each arm is based on.
pub(crate) fn expandable_primitive_identity(
    primitive: ExpandablePrimitive,
) -> (String, Option<i64>) {
    use ExpandablePrimitive as P;
    match primitive {
        // TeX82 §25 dispatches `\expandafter`/`\csname`/`\endcsname` through
        // their own dedicated command codes; `CommandIdentity::from_meaning`
        // (crates/tex-command/src/command.rs) already intercepts these
        // before this function is reached from `CurrentCommand`, but the
        // match here must stay exhaustive independent of that caller.
        P::ExpandAfter => ("expand_after".into(), Some(0)),
        P::CsName => ("cs_name".into(), Some(0)),
        P::EndCsName => ("end_cs_name".into(), Some(0)),
        P::NoExpand => ("no_expand".into(), Some(0)),
        // `convert`'s selector: number=0, roman_numeral=1, string=2,
        // meaning=3, font_name=4, job_name=5 (`CommandIdentity::Convert`
        // already intercepts these before this function is reached from
        // `CurrentCommand`; kept here for the same exhaustiveness reason).
        P::Number => ("convert".into(), Some(0)),
        P::RomanNumeral => ("convert".into(), Some(1)),
        P::String => ("convert".into(), Some(2)),
        P::Meaning => ("convert".into(), Some(3)),
        P::FontName => ("convert".into(), Some(4)),
        P::JobName => ("convert".into(), Some(5)),
        P::The => ("the".into(), Some(0)),
        P::Input => ("input".into(), Some(0)),
        // tex.web: `primitive("endinput",input,1)`.
        P::EndInput => ("input".into(), Some(1)),
        // e-TeX: `primitive("scantokens",input,2)`.
        P::Scantokens => ("input".into(), Some(2)),
        // `top_bot_mark`'s selector: top_mark=0, first_mark=1, bot_mark=2,
        // split_first_mark=3, split_bot_mark=4; e-TeX's `\...marks` variants
        // add `marks_code`=5 to each.
        P::TopMark => ("top_bot_mark".into(), Some(0)),
        P::FirstMark => ("top_bot_mark".into(), Some(1)),
        P::BotMark => ("top_bot_mark".into(), Some(2)),
        P::SplitFirstMark => ("top_bot_mark".into(), Some(3)),
        P::SplitBotMark => ("top_bot_mark".into(), Some(4)),
        P::TopMarks => ("top_bot_mark".into(), Some(5)),
        P::FirstMarks => ("top_bot_mark".into(), Some(6)),
        P::BotMarks => ("top_bot_mark".into(), Some(7)),
        P::SplitFirstMarks => ("top_bot_mark".into(), Some(8)),
        P::SplitBotMarks => ("top_bot_mark".into(), Some(9)),
        // `if_test`'s selector: if_char=0 .. if_case=16 (tex.web), then
        // e-TeX's if_def_code=17, if_cs_code=18, if_font_char_code=19.
        P::If => ("if_test".into(), Some(0)),
        P::IfCat => ("if_test".into(), Some(1)),
        P::IfNum => ("if_test".into(), Some(2)),
        P::IfDim => ("if_test".into(), Some(3)),
        P::IfOdd => ("if_test".into(), Some(4)),
        P::IfVMode => ("if_test".into(), Some(5)),
        P::IfHMode => ("if_test".into(), Some(6)),
        P::IfMMode => ("if_test".into(), Some(7)),
        P::IfInner => ("if_test".into(), Some(8)),
        P::IfVoid => ("if_test".into(), Some(9)),
        P::IfHBox => ("if_test".into(), Some(10)),
        P::IfVBox => ("if_test".into(), Some(11)),
        P::IfX => ("if_test".into(), Some(12)),
        P::IfEof => ("if_test".into(), Some(13)),
        P::IfTrue => ("if_test".into(), Some(14)),
        P::IfFalse => ("if_test".into(), Some(15)),
        P::IfCase => ("if_test".into(), Some(16)),
        P::IfDefined => ("if_test".into(), Some(17)),
        P::IfCsName => ("if_test".into(), Some(18)),
        P::IfFontChar => ("if_test".into(), Some(19)),
        // BEST-EFFORT: e-TeX's own if_test extension order is not confirmed
        // past if_font_char_code; kept sequential and distinguishable.
        P::IfInCsName => ("if_test".into(), Some(20)),
        // `fi_or_else`: fi_code=2, else_code=3, or_code=4.
        P::Fi => ("fi_or_else".into(), Some(2)),
        P::Else => ("fi_or_else".into(), Some(3)),
        P::Or => ("fi_or_else".into(), Some(4)),
        // e-TeX's inaccessible outer end-template command; already
        // intercepted before `canonical_command_identity`'s generic
        // `Meaning::ExpandablePrimitive` arm reaches this function, but kept
        // here for exhaustiveness.
        P::EndTemplate => ("end_template".into(), Some(249_988)),
        // e-TeX: `primitive("unexpanded",the,1)`,
        // `primitive("detokenize",the,show_tokens)`, show_tokens=5.
        P::Unexpanded => ("the".into(), Some(1)),
        P::Detokenize => ("the".into(), Some(5)),
        // e-TeX: `primitive("unless",expand_after,1)`.
        P::Unless => ("expand_after".into(), Some(1)),
        // e-TeX: `eTeX_revision_code=etex_convert_base=5`; e-TeX's own
        // change file also shifts `job_name_code` from 5 to 6 to make room
        // (`/tmp/etex.ch` line 1367), which this classifier does not model
        // per-dialect (`JobName` above keeps the TeX82 value, correct for
        // the only fixtures wired in today; see the module documentation).
        P::ETeXRevision => ("convert".into(), Some(5)),
        // Never constructed: `\eTeXversion` is installed as
        // `Meaning::InternalInteger`, not this variant (see
        // `crates/tex-expand/src/lib.rs`'s e-TeX table). Kept only for
        // exhaustiveness.
        P::ETeXVersion => ("last_item".into(), None),
        // pdfTeX's message-style expansion primitive: BEST-EFFORT, not
        // confirmed against a live pdfTeX reference (see the module
        // documentation).
        P::Expanded => ("convert".into(), Some(6)),
        // Engine-neutral Umber/LaTeX-extension additions with no tex.web or
        // e-TeX analog; BEST-EFFORT placeholders in the `convert` family
        // they most resemble (each expands to text).
        P::FileSize => ("convert".into(), Some(7)),
        P::StringCompare => ("convert".into(), Some(8)),
        P::ShellEscape => ("convert".into(), Some(9)),
        P::CreationDate => ("convert".into(), Some(10)),
        // BEST-EFFORT pdfTeX-only expandable primitives, none exercised by
        // any committed fixture; see the module documentation.
        P::PdfTeXRevision => ("convert".into(), Some(11)),
        P::PdfTeXBanner => ("convert".into(), Some(12)),
        P::PdfFontSize => ("convert".into(), Some(13)),
        P::LeftMarginKern => ("convert".into(), Some(14)),
        P::RightMarginKern => ("convert".into(), Some(15)),
        P::PdfFontName => ("convert".into(), Some(16)),
        P::PdfFontObjectNumber => ("convert".into(), Some(17)),
        P::PdfInsertHeight => ("convert".into(), Some(18)),
        P::PdfXImageBBox => ("convert".into(), Some(19)),
        P::PdfColorStackInit => ("convert".into(), Some(20)),
        P::PdfXFormName => ("convert".into(), Some(21)),
        P::PdfPageRef => ("convert".into(), Some(22)),
        // Resolves a control sequence's original (pre-`\let`) primitive
        // meaning before a nested expansion, similar in spirit to
        // `\expandafter`'s family.
        P::PdfPrimitive => ("expand_after".into(), Some(2)),
        P::IfPdfPrimitive => ("if_test".into(), Some(21)),
        P::IfPdfAbsNum => ("if_test".into(), Some(22)),
        P::IfPdfAbsDim => ("if_test".into(), Some(23)),
        P::PdfEscapeString => ("convert".into(), Some(23)),
        P::PdfEscapeName => ("convert".into(), Some(24)),
        P::PdfEscapeHex => ("convert".into(), Some(25)),
        P::PdfUnescapeHex => ("convert".into(), Some(26)),
        P::PdfFileModificationDate => ("convert".into(), Some(27)),
        P::PdfMdFiveSum => ("convert".into(), Some(28)),
        P::PdfFileDump => ("convert".into(), Some(29)),
        P::PdfMatch => ("convert".into(), Some(30)),
        P::PdfLastMatch => ("convert".into(), Some(31)),
        P::PdfUniformDeviate => ("convert".into(), Some(32)),
        P::PdfNormalDeviate => ("convert".into(), Some(33)),
    }
}
