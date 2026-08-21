//! Borrow-barrier values for uncommon main-control operations.
//!
//! These values are runtime-only and never own an interpreter or semantic
//! state. Ranked commands must not acquire a variant here.

use super::super::*;

#[derive(Clone)]
pub(in crate::main_control) enum ColdOperation<G> {
    Continue,
    Relax,
    /// e-TeX 2.6 `etex.ch` [17.3822--3880]'s `hmode+valign` extension:
    /// the four nonzero `valign` modifiers are text-direction nodes when
    /// `\TeXXeTstate>0`; otherwise `eTeX_enabled` diagnoses and ignores them.
    TextDirection {
        direction: tex_state::node::Direction,
        enabled: bool,
    },
    AlignPeekRestart {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §1128's `abs(align_state)>2` recovery: report the delivered
    /// delimiter and drop it without a backup or inserted brace.
    MisplacedAlignmentDelimiter {
        token: Token,
        context: String,
    },
    /// TeX82 §1129's command-specific misplaced-alignment report.
    MisplacedAlignmentCommand {
        omit: bool,
    },
    /// TeX82 §342 has just run §789's
    /// ``@<Insert the ⟨v_j⟩ template and |goto restart|@>``.
    ///
    /// This is not a `main_control` step at all: §789 runs *inside* `get_next`
    /// and jumps back to its own `restart` label, so the fetch that triggered
    /// it is still in progress. Umber routes the push out to the executor
    /// because the alignment's identity lives here, but must not treat the
    /// round trip as a return to §1030's `big_switch` -- main control is still
    /// parked wherever it was, and §1030's two fetch labels disagree about the
    /// `end_template` that a ⟨v_j⟩ template is about to deliver: §380's
    /// `get_x_token` rewrites it to `endv` in place, while §1038's `x_token`
    /// reaches §366 `expand` and §375 backs up a separate `frozen_endv` token
    /// to be reread.
    AlignmentTemplateEntered,
    /// TeX82 §1132's `align_group` recovery has backed up the closing brace
    /// and inserted frozen `\cr`; §82's `ins_error` diagnostic must run
    /// before alignment delivery fetches that inserted row terminator.
    MissingAlignmentCr,
    MissingMathShift,
    ReplayCompleted(tex_command::CommandReplayEpisode),
    Math(MathRequest),
    DisplayAlignmentRecovery,
    MathDelimiter(MathDelimiterBoundary),
    MathFamily {
        family: tex_command::ScannedMathFamily,
        font: FontId,
        global: bool,
    },
    EndOfInput,
    /// TeX82 §1045's `vmode+stop: if its_all_over then return` -- the only
    /// exit from `main_control`. §1335's `final_cleanup` has already unwound
    /// the abandoned input stack, so the job-termination effect is this
    /// step's, not a later input-exhaustion step's.
    ///
    /// §1335's `c` -- 0 for `\end`, 1 for `\dump` -- selects the whole tail
    /// of `final_cleanup`, so it is carried rather than discarded.
    End {
        dump: bool,
        incomplete_conditions: Vec<tex_command::IncompleteCondition>,
    },
    /// TeX82 §1051's `privileged` failure for `\\end`/`\\dump` in internal
    /// vertical mode (`mode<0`): `report_illegal_case`, and the job keeps
    /// running. Shares the recovery shape of `IllegalBoxShift` and friends.
    IllegalStop {
        token: Token,
    },
    /// TeX82 §1045's `any_mode(mac_param): report_illegal_case`. A bare
    /// parameter command is diagnosed and discarded in every mode; it does
    /// not terminate main control.
    IllegalMacroParameter {
        token: Token,
    },
    /// TeX82 §1135's `cs_error`: a stray `\endcsname` is diagnosed and
    /// ignored exactly once, without changing input or mode state.
    ExtraEndCsName,
    /// TeX82 §1054's `its_all_over` false branch: the backed-up stop stays
    /// live while `\\hbox to \\hsize{}`, `\\vfill`, and
    /// `\\penalty-'10000000000` are appended to the contribution list and
    /// §994's `build_page` runs. Whether that fires `\\output` is §1005's
    /// decision.
    EjectResidualPage,
    Count {
        index: u16,
        value: i32,
        global: bool,
    },
    Dimen {
        index: u16,
        value: Scaled,
        global: bool,
    },
    /// TeX82 §1055's `assign_box_dimen: alter_box_dimen(cur_chr)` (`\wd`,
    /// `\ht`, `\dp`): `scan_eight_bit_int`, `scan_optional_equals`, then
    /// `scan_normal_dimen` set the given dimension on the box register's
    /// visible node if it is not void; a void or absent register is a
    /// documented no-op (§1055 skips the mutation entirely rather than
    /// erroring), matched by `Universe::set_box_dimension`'s own early
    /// return.
    BoxDimensionAssignment {
        index: u16,
        dimension: tex_state::BoxDimension,
        value: Scaled,
        global: bool,
    },
    Skip {
        index: u16,
        value: GlueSpec,
        source_identity: Option<tex_state::GlueId<G>>,
        source_skip_index: Option<u16>,
        redundant: bool,
        reassigning: bool,
        global: bool,
    },
    Muskip {
        index: u16,
        value: GlueSpec,
        source_identity: Option<tex_state::GlueId<G>>,
        redundant: bool,
        reassigning: bool,
        global: bool,
    },
    HorizontalSkip {
        value: GlueSpec,
    },
    VerticalSkip {
        value: GlueSpec,
    },
    Kern {
        amount: Scaled,
    },
    /// TeX82 §1102's `any_mode(break_penalty): append_penalty` -- `\penalty`
    /// is legal in every mode (vertical, horizontal, and math alike) with no
    /// mode switch of its own, unlike `\hskip`/`\kern`'s vertical-mode
    /// paragraph-start recovery above.
    Penalty {
        amount: i32,
    },
    CharacterCode {
        value: i32,
        origin: tex_state::token::OriginId,
        suppress_left_boundary: bool,
    },
    /// TeX82 §1105's `any_mode(remove_item): delete_last` -- `\unpenalty`,
    /// `\unkern`, and `\unskip` are legal in every mode with no scan of their
    /// own (the removed node, if any, is selected purely by matching the
    /// primitive against the current list's tail).
    DeleteLast {
        primitive: UnexpandablePrimitive,
        context: String,
    },
    /// TeX82 §1264's `new_interaction`: `\batchmode`/`\nonstopmode`/
    /// `\scrollmode`/`\errorstopmode` carry no operand of their own -- the
    /// target `InteractionMode` is selected from the delivered primitive at
    /// apply time, mirroring `DeleteLast` above.
    SetInteractionMode(UnexpandablePrimitive),
    /// e-TeX 2.6 etex.ch §3736's assignable `\interactionmode` primitive.
    SetInteractionModeValue {
        value: i32,
        context: String,
    },
    /// TeX82 §1112's `hmode+ital_corr: append_italic_correction` (the
    /// procedure itself is §1113) or its math-mode twin (§1112's
    /// `mmode+ital_corr: tail_append(new_kern(0))`); which applies is
    /// resolved from the live mode at apply time since neither takes an
    /// operand. Vertical mode is instead `IllegalItalicCorrection` below
    /// (`\/` is one of §1111's "Forbidden cases", not a paragraph-starting
    /// command).
    ItalicCorrection,
    /// TeX82 §1111's "Forbidden cases" `you_cant`/`report_illegal_case`
    /// recovery for `\/` in vertical or internal-vertical mode
    /// (`vmode+ital_corr`).
    IllegalItalicCorrection {
        token: Token,
    },
    /// TeX82 §1038's lookahead consumes `no_boundary` after a character run
    /// by setting `bchar:=non_char`, suppressing only that run's right
    /// boundary processing. The §1030 big-switch occurrence has different
    /// semantics and is folded into its following command during scanning.
    /// §1045's math-mode occurrence is a no-op.
    NoBoundary {
        suppress_right: bool,
    },
    /// TeX82 §1171's `mmode+non_script: tail_append(new_glue(zero_glue));
    /// subtype(tail):=cond_math_glue`. Legal only in math/display-math mode;
    /// §1046's `non_math(non_script)` routes every other mode through
    /// `MissingMathShift` instead (`scan_command` selects between the two
    /// before this step is produced).
    NonScript,
    /// TeX82 §1030's `hmode+ex_space,mmode+ex_space: goto append_normal_space`
    /// and §1090's `vmode+ex_space: back_input; new_graf(true)` -- `\ ` (the
    /// explicit control-space primitive) starts a paragraph in vertical mode
    /// exactly like a letter, then always appends the plain interword glue
    /// regardless of the current space factor.
    ControlSpace,
    /// TeX82 §1243's `set_aux` assignment (`alter_aux`) for the vertical-mode
    /// modifier: `\prevdepth=<dimen>` sets the enclosing vertical list's
    /// `prev_depth`, the field `append_to_vlist` (§679) reads to decide
    /// baselineskip/lineskip insertion before the next box. Legal only in
    /// vertical or internal-vertical mode; §1243's `report_illegal_case`
    /// (`abs(mode)<>vmode`) otherwise leaves the value alone.
    PrevDepth {
        value: Scaled,
    },
    /// TeX82 §1243 checks `cur_chr<>abs(mode)` before calling either
    /// `scan_optional_equals` or `scan_normal_dimen`. Thus an illegal-mode
    /// `\prevdepth` reports `report_illegal_case` while preserving the very
    /// next token as an ordinary main-control command.
    IllegalPrevDepth {
        token: Token,
    },
    /// TeX82 §1243's `set_aux` assignment for the horizontal-mode modifier:
    /// `\spacefactor=<number>` sets the current horizontal list's space
    /// factor. Legal only in horizontal or restricted-horizontal mode;
    /// §1243's `report_illegal_case` (`abs(mode)<>hmode`) otherwise leaves
    /// the value alone. A scanned value outside `1..32767` is a "Bad space
    /// factor" diagnostic that likewise leaves the space factor unchanged.
    SpaceFactor {
        value: i32,
    },
    /// TeX82 §1243 checks `cur_chr<>abs(mode)` before calling either
    /// `scan_optional_equals` or `scan_int`. Thus an illegal-mode
    /// `\spacefactor` reports `report_illegal_case` while preserving the
    /// very next token as an ordinary main-control command.
    IllegalSpaceFactor {
        token: Token,
    },
    /// TeX82 §1244's `set_prev_graf` (`alter_prev_graf`): `\prevgraf=<number>`
    /// is `any_mode` and walks the mode nest up to its nearest enclosing
    /// vertical level (`while abs(nest[p].mode_field)<>vmode do decr(p)`),
    /// setting that level's `prev_graf` (paragraph count so far) directly --
    /// unlike `\spacefactor`/`\prevdepth`, it never reports an illegal case. A
    /// negative scanned value is a "Bad \prevgraf" diagnostic that leaves the
    /// count unchanged.
    PrevGraf {
        value: i32,
    },
    /// TeX82 §1242's `set_page_dimen: alter_page_so_far`, whose body is
    /// §1245: `c:=cur_chr; scan_optional_equals; scan_normal_dimen;
    /// page_so_far[c]:=cur_val`. This is `\pagegoal`, `\pagetotal`,
    /// `\pagestretch`, `\pagefilstretch`, `\pagefillstretch`,
    /// `\pagefilllstretch`, `\pageshrink`, and `\pagedepth`.
    ///
    /// There is deliberately no `global` field: §1242 states outright that
    /// "these definitions are always global", and `page_so_far` is a plain
    /// engine array rather than an `eqtb` entry, so neither the `\global`
    /// prefix nor `\globaldefs` can reach it and no save-stack entry is
    /// pushed. `PrevDepth`/`SpaceFactor`/`PrevGraf` above and
    /// `BoxDimensionAssignment` below are scoped identically for the same
    /// reason.
    PageDimension {
        dimension: PageDimension,
        value: Scaled,
    },
    /// TeX82 §1242's `set_page_int: alter_integer`, whose body is §1246:
    /// `c:=cur_chr; scan_optional_equals; scan_int; if c=0 then
    /// dead_cycles:=cur_val else insert_penalties:=cur_val`. This is
    /// `\deadcycles` and `\insertpenalties`.
    ///
    /// Unscoped for the same §1242 reason as `PageDimension` above.
    PageInteger {
        integer: PageInteger,
        value: i32,
    },
    FixedHorizontalGlue {
        primitive: UnexpandablePrimitive,
    },
    FixedVerticalGlue {
        primitive: UnexpandablePrimitive,
    },
    ParagraphIndent {
        indent: bool,
    },
    ParagraphShape {
        lines: Vec<ParagraphShapeLine>,
        global: bool,
    },
    PenaltyArray {
        kind: PenaltyArrayKind,
        values: Vec<i32>,
        global: bool,
    },
    Toks {
        index: u16,
        tokens: tex_command::AttemptTokenListId,
        global: bool,
    },
    IntParam {
        index: u16,
        value: i32,
        global: bool,
    },
    DimenParam {
        index: u16,
        value: Scaled,
        global: bool,
    },
    TokParam {
        index: u16,
        tokens: Option<tex_command::AttemptTokenListId>,
        global: bool,
    },
    GlueParam {
        index: u16,
        value: GlueSpec,
        global: bool,
    },
    CodeTable {
        primitive: UnexpandablePrimitive,
        character: char,
        value: i32,
        global: bool,
    },
    /// pdftex.web §§468--470's globally owned per-font byte-code assignment.
    PdfFontCode {
        table: tex_state::PdfFontCode,
        font: FontId,
        character: u8,
        value: i32,
    },
    /// pdftex.web §471 suppresses ligature construction for one live font.
    PdfNoLigatures {
        font: FontId,
    },
    FontSelect {
        font: FontId,
        selector: Option<Symbol>,
        global: bool,
    },
    FontDefinition {
        request: FontLoadRequest,
        resource: Box<Option<FontResource>>,
        global: bool,
    },
    GeneratedFontDefinition {
        definition: ScannedGeneratedFontDefinition,
        global: bool,
    },
    InputStream {
        request: InputStreamRequest,
        resource: Option<SourceRegistration>,
    },
    PdfXImage {
        request: PdfImageRequest,
        resource: PdfImageResource,
    },
    PdfRefXImage {
        object: i32,
    },
    /// pdftex.web §1585's `\pdfsetrandomseed`: the command scanner has
    /// consumed one ordinary integer and normalized its sign; application
    /// replaces the ungrouped job RNG state atomically.
    PdfSetRandomSeed {
        seed: i32,
    },
    /// pdftex.web §1586's `\pdfresettimer`: there is no operand; application
    /// atomically rebases the ungrouped job timer to the deterministic
    /// monotonic sample already held by `World`.
    PdfResetTimer,
    /// pdftex.web §§1594–1596's operand-free interword-space controls.
    /// Application appends an ordered whatsit after flushing any pending
    /// horizontal character run; shipout traversal owns the toggle state.
    PdfInterwordSpace(tex_state::node::PdfAccessibilityControl),
    /// pdftex.web §§1597–1598's operand-free running-link shipout controls.
    /// Application appends an ordered whatsit; PDF traversal owns the
    /// initially-enabled policy for continuation annotations.
    PdfRunningLink(bool),
    /// pdftex.web §1599's expanded balanced text selecting the global,
    /// job-owned fallback font name used by accessible-space PDF output.
    PdfSpaceFont(tex_command::AttemptTokenListId),
    PdfGraphics(PdfGraphicsRequest),
    PdfObject(PdfObjectRequest),
    PdfReferenceObject(PdfReferenceObjectRequest),
    PdfForm(PdfFormRequest),
    PdfDocumentFragment(PdfDocumentFragmentRequest),
    PdfNavigation(PdfNavigationRequest),
    FontDimen {
        font: FontId,
        /// tex.web §578's `n`, unrecovered. A number at or below zero
        /// resolves to §578's scratch `fmem_ptr` and is reported by §579, so
        /// the scan must carry it rather than reject it.
        number: i32,
        value: Scaled,
        /// §82's `show_context` at the point §578 decided the number was
        /// unusable -- after the font identifier, before `=<dimen>`. `None`
        /// when §578 accepted it, which is also what says no §579 report is
        /// due.
        recovery_context: Option<String>,
    },
    FontInteger {
        font: FontId,
        skew: bool,
        value: i32,
    },
    /// pdftex.web §§1680--1682's font expansion configuration.
    PdfFontExpand {
        font: FontId,
        spec: tex_typeset::expansion::FontExpansionSpec,
    },
    /// pdftex.web §§1601--1607's font and map extension actions.  Operand
    /// expansion belongs to the command processor; host-neutral state
    /// mutation remains at the apply seam.
    PdfFontAction {
        primitive: UnexpandablePrimitive,
        font: Option<FontId>,
        first: Option<tex_command::AttemptTokenListId>,
        second: Option<tex_command::AttemptTokenListId>,
    },
    DeferredOpenOut {
        stream: u8,
        file_name: String,
    },
    DeferredCloseOut {
        stream: tex_command::WriteStreamSelector,
    },
    DeferredWrite {
        stream: tex_command::WriteStreamSelector,
        tokens: tex_command::AttemptTokenListId,
    },
    DeferredSpecial {
        deferred: bool,
        tokens: tex_command::AttemptTokenListId,
    },
    /// TeX82 §1377's `@<Implement \setlanguage@>`, reached from §1348's
    /// `do_extension` on `set_language_code` (§1344's `extension` modifier
    /// 5). Unlike §1376's `fix_language`, which appends a §1341
    /// `language_node` only when the language actually changes, §1377
    /// appends one unconditionally -- `\setlanguage` is an explicit request,
    /// so a same-language `\setlanguage` still produces a whatsit.
    ///
    /// `language` is `cur_val` exactly as `scan_int` left it; §1377's own
    /// `<=0`/`>255` normalization to `clang` is performed at the apply seam
    /// together with `norm_min` (§1091), because both write mode-nest and
    /// list state the scan phase does not own.
    SetLanguage {
        language: i32,
    },
    /// TeX82 §1377's `if abs(mode)<>hmode then report_illegal_case`.
    ///
    /// The mode test precedes both `new_whatsit` and `scan_int`, so an
    /// out-of-mode `\setlanguage` scans no operand at all -- matching
    /// `IllegalBoxShift`/`IllegalInsertOrAdjust`'s same-shaped recovery, and
    /// unlike `\prevdepth`'s §1243 check, which runs after its value is
    /// scanned.
    IllegalSetLanguage {
        token: Token,
    },
    Arithmetic {
        primitive: UnexpandablePrimitive,
        target: ArithmeticTarget,
        operand: ArithmeticOperand,
        global: bool,
    },
    /// TeX82 §1236's recoverable invalid-target return from
    /// `do_register_command`. The target command has been consumed, but no
    /// operand is scanned and no value is changed.
    InvalidArithmeticTarget {
        primitive: UnexpandablePrimitive,
        target: tex_command::PrintCommand<G>,
    },
    CharacterDefinition {
        primitive: UnexpandablePrimitive,
        target: Symbol,
        /// Meaning replaced by §1224's already-applied provisional `\relax`.
        provisional_old: tex_state::meaning::ResolvedMeaning<G>,
        /// `cur_val` after §434/§436's recover-to-zero.
        value: i32,
        global: bool,
    },
    /// TeX82 §1252's `hyph_data` command: `\patterns` (`chr_code=1`) installs
    /// pattern data through §960's `new_patterns`; `\hyphenation`
    /// (`chr_code=0`) installs exception words through §934's
    /// `new_hyph_exceptions`. The scan carries §935's raw exception words or
    /// §962's normalized pattern specs; the flag selects which table they
    /// populate.
    HyphenationData {
        words: Vec<Vec<char>>,
        pattern_specs: Vec<tex_state::hyphenation::PatternSpec>,
        patterns: bool,
        /// §82's context for whichever of the two `\patterns` rejections the
        /// apply seam raises. Both report before the braced group is read --
        /// §960's `trie_not_ready=false` branch before §473 discards it, and
        /// §1252's production branch before its own `repeat get_token` flush --
        /// so the context has to be captured at the pre-scan cursor, with
        /// `\patterns` behind it and the group still ahead.
        rejection_context: String,
        /// Whether §960's trie was already built, which is the half of the
        /// rejection test the command core can see. The other half -- whether
        /// tex.web's `init`/`tini` split would have produced this binary at
        /// all -- is the session's, and is applied with it.
        trie_built: bool,
    },
    RegisterDefinition {
        primitive: UnexpandablePrimitive,
        target: Symbol,
        /// Meaning replaced by §1224's already-applied provisional `\relax`.
        provisional_old: tex_state::meaning::ResolvedMeaning<G>,
        index: u16,
        global: bool,
    },
    AfterGroup(tex_state::token::TracedTokenWord),
    AfterAssignment(tex_state::token::TracedTokenWord),
    Rule {
        width: Option<Scaled>,
        height: Option<Scaled>,
        depth: Option<Scaled>,
        horizontal: bool,
    },
    HRuleHereExceptLeaders,
    Message {
        tokens: tex_command::AttemptTokenListId,
        error: bool,
    },
    DisplayDiagnostic(ScannedDisplayDiagnostic),
    ShowBox {
        index: u16,
    },
    /// TeX82 §1293's `show_lists_code` branch of `show_whatever`, which takes
    /// no operand: `begin_diagnostic; show_activities`. The mode is carried
    /// so the committed effect can name the nest it reported.
    ShowLists,
    /// e-TeX 2.6 `etex.ch` [17.3623--3671]'s `\\showtokens`: command
    /// processing has removed the compulsory braces and frozen the
    /// unexpanded balanced interior. Replay owns only diagnostic rendering.
    ShowTokens {
        tokens: tex_command::AttemptTokenListId,
    },
    /// e-TeX 2.6 `etex.ch` [17.3703--3732]'s read-only conditional-stack
    /// diagnostic, detached in innermost-to-outermost traversal order.
    ShowIfs {
        conditions: Vec<tex_command::ActiveCondition>,
    },
    /// e-TeX 2.6 [49.1292]'s operand-free, any-mode `\showgroups`.
    ShowGroups {
        diagnostic: Option<crate::diagnostics::ShowGroupsDiagnostic>,
    },
    VSplit(ScannedVSplit),
    ImmediateExtension(ImmediateExtension),
    BoxRegister {
        index: u16,
        copy: bool,
        ships_out: bool,
    },
    Unbox {
        primitive: UnexpandablePrimitive,
        index: u16,
        error_context: String,
    },
    /// e-TeX 2.6 `etex.ch` [45.999]'s operand-free extensions of TeX82's
    /// `un_vbox` command. The selected saved list is detached and spliced
    /// into the current list atomically; unlike `\unvbox`, no register
    /// number is scanned.
    SavedVerticalDiscards(UnexpandablePrimitive),
    LastBox {
        error_context: String,
    },
    Leaders {
        kind: GlueKind,
        payload: LeaderPayload,
        glue: GlueSpec,
    },
    LeaderRegister {
        kind: GlueKind,
        index: u16,
        copy: bool,
        glue: GlueSpec,
    },
    MissingLeaderPayload,
    LeadersNotFollowedByGlue,
    BeginShipout,
    BeginAlignment {
        vertical: bool,
        owner: Option<tex_state::interner::Symbol>,
    },
    AlignmentPreambleOpening {
        alignment: AlignmentIdentity,
        packing: ScannedPackingSpec,
    },
    AlignmentPreambleStart {
        alignment: AlignmentIdentity,
    },
    AlignmentCellOpening {
        alignment: AlignmentIdentity,
        opening: AlignmentCellOpening,
    },
    /// TeX82's `do_endv` completed a command-owned v-template.  Applying
    /// this result retires that exact frame before the backed-up delimiter
    /// resumes through `get_next`.
    AlignmentCellFinish {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §37 delivered the alignment-closing right brace, so `fin_align`
    /// must complete before the outer suspended delivery context resumes.
    AlignmentFinish {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §37 has consumed `\\noalign` and its compulsory opening brace.
    /// Command control owns both deliveries; the executor now owns the
    /// no-align group's structural entry.
    BeginNoAlign {
        alignment: AlignmentIdentity,
    },
    /// TeX82 §1127's `align_error` has backed up the delimiter and inserted
    /// this balancing brace. Applying the step emits the matching `ins_error`
    /// diagnostic before replay fetches the inserted token.
    AlignmentRecovery {
        brace: Catcode,
    },
    BeginSimpleGroup,
    EndSimpleGroup,
    /// TeX82 §1068's `handle_right_brace` for a brace the current group
    /// cannot account for. `forgotten` is §1069's `case cur_group of` -- the
    /// opener the brace was probably standing in for -- and `None` is §1068's
    /// own `bottom_level` arm, "Too many }'s".
    ExtraRightBrace {
        forgotten: Option<ForgottenGroupOpener>,
    },
    /// TeX82 §1186: the closing brace of a `math_group` opened by §1153's
    /// `push_math`, or §1174's `build_choices` closing a `math_choice_group`
    /// opened by §1172/§1174. Applying it only `unsave`s; the nested loop in
    /// `execute_live_math_group` notices the level is gone and finishes the
    /// mlist.
    EndMathGroup(GroupKind),
    /// TeX82 §1064's general `off_save`: the innermost group could not
    /// accommodate the scanned command, so `scan_off_save` already chose and
    /// inserted the matching closer ahead of the backed-up command
    /// (`CommandProcessor::recover_off_save`). The execute phase only prints
    /// §1064's report naming what was inserted; the closer travels as the
    /// typed choice §1065 already made rather than as rendered text, because
    /// two of the four spell a control sequence and so must be printed
    /// through §63's `print_esc` under the live `\\escapechar`.
    OffSave(OffSaveCloser),
    /// TeX82 §§1064/1066's bottom-level `off_save`: no enclosing group
    /// existed to close, so the offending command was dropped outright
    /// (`CommandProcessor::report_off_save_bottom_drop` already ran); the
    /// execute phase prints "Extra `<command>`" naming its own spelling.
    OffSaveBottomDrop {
        token: Token,
    },
    OutputRoutineOpeningBrace,
    EndOutputRoutine,
    AlignmentPeekCell {
        alignment: AlignmentIdentity,
        omit: bool,
    },
    NoAlignEndGroup {
        alignment: AlignmentIdentity,
    },
    SetBox {
        target: SetBoxTarget,
        path: ScannedSetBoxPath,
    },
    BeginBox(ScannedBoxConstruction),
    BeginLeaderBox {
        construction: ScannedBoxConstruction,
        kind: GlueKind,
    },
    /// TeX82 §1073's box-shift prefixes (`\raise`, `\lower`, `\moveleft`,
    /// `\moveright`): the already-signed shift amount (tex.web's
    /// `box_context`) plus `scan_box`'s own `make_box` operand (§1084).
    /// `ScannedBoxShiftPayload::Construction` shares the `BeginBox`/
    /// `BoxEndGroup` body-closing machinery; the other
    /// variants resolve to a node immediately, exactly like `\box<n>`,
    /// `\lastbox`, and `\vsplit` do outside a shift.
    BoxShift(ScannedBoxShift),
    /// TeX82's "Forbidden cases" `you_cant`/`report_illegal_case` recovery
    /// for a box-shift prefix used in the wrong mode (`vmode+vmove`,
    /// `hmode+hmove`, `mmode+hmove`): the dimension is never scanned, unlike
    /// `\prevdepth`'s mode check, which runs only after its value is
    /// scanned.
    IllegalBoxShift {
        token: Token,
    },
    /// TeX82 §1099's `begin_insert_or_adjust` for `\insert` and `\vadjust`.
    /// The class-number bound check (0..=255) and the reserved-255 recovery
    /// both need `Universe` diagnostics, so replay receives only the raw
    /// scanned class (fixed at 255 for `\vadjust`); the body's mandatory
    /// opening brace was already consumed by `scan_left_brace`. It shares the
    /// same brace-matching machinery as `BeginBox`/`BoxEndGroup`.
    BeginInsert(ScannedInsertConstruction),
    /// TeX82's "Forbidden cases" `vmode+vadjust`: `\vadjust` never reaches
    /// its mandatory `scan_left_brace` in vertical mode, matching
    /// `IllegalBoxShift`/`IllegalItalicCorrection`'s same-shaped recovery.
    IllegalInsertOrAdjust {
        token: Token,
    },
    /// TeX82 §1144's `@<Forbidden cases@>=non_math(eq_no)` (added to the
    /// shared Forbidden-cases list first built at §1048): `\eqno`/`\leqno`
    /// outside math mode take `report_illegal_case` ("You can't use
    /// `\eqno' in ... mode") rather than §1047's `insert_dollar_sign`, even
    /// though tex.web registers them under the same `eq_no` command code as
    /// the math-request vocabulary `scan_math_request` otherwise
    /// dispatches. Reaching this arm proves `mode` is not
    /// `Math`/`DisplayMath` (that gate would have consumed the primitive
    /// first via `Request::EquationNumber`), matching
    /// `IllegalBoxShift`/`IllegalItalicCorrection`/`IllegalInsertOrAdjust`'s
    /// same-shaped recovery. `mmode+eq_no` itself (gated by
    /// `privileged`/`cur_group`) is unaffected.
    IllegalEqNo {
        token: Token,
    },
    /// TeX82 §§1051 and 1130's `mmode+halign: if privileged ...`:
    /// inline math has negative `mmode`, so `privileged` diagnoses the
    /// command and returns before `init_align` scans any preamble input.
    IllegalHAlign {
        token: Token,
    },
    /// TeX82 §1048's `@<Forbidden cases@>=...,any_mode(last_item),...` (the
    /// same module `IllegalBoxShift`'s `vmode+vmove`/`hmode+hmove`/
    /// `mmode+hmove` triple comes from, and that `IllegalEqNo`'s §1144
    /// addition later extends): `\lastpenalty`, `\lastkern`, and `\lastskip`
    /// have no assignment form and no standalone typesetting meaning in any
    /// mode -- they are legal only as an internal-value operand inside a
    /// scan (`CommandProcessor::internal_value_from_command`'s `LastPenalty`/
    /// `LastKern`/`LastSkip` arms). Reaching main control with one of these
    /// as the delivered command therefore always means `report_illegal_case`,
    /// matching `IllegalBoxShift`/`IllegalInsertOrAdjust`/`IllegalEqNo`'s
    /// same-shaped recovery.
    IllegalLastItem {
        token: Token,
        context: String,
    },
    BoxEndGroup {
        ships_out: bool,
    },
    /// TeX82 §1101 and e-TeX 2.6 `etex.ch` [26.424]'s `make_mark`: a fully
    /// expanded balanced general text, appended as the selected mark class.
    Mark {
        class: u16,
        tokens: tex_command::AttemptTokenListId,
    },
    Paragraph,
    MathShift {
        pairing: MathShiftPairing,
    },
    ParagraphStart,
    Character {
        ch: char,
        cat: Catcode,
        origin: tex_state::token::OriginId,
        suppress_left_boundary: bool,
    },
    Accent(ScannedAccent),
    DiscretionaryOpening(ScannedDiscretionaryOpening),
    DiscretionaryPartEnd,
    DiscretionaryHyphen {
        origin: tex_state::token::OriginId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::main_control) enum MathShiftPairing {
    Unpaired,
    Paired,
    ProbeDisplayEnd,
}

impl<G> ColdOperation<G> {
    pub(in crate::main_control) const fn fires_afterassignment(&self) -> bool {
        matches!(
            self,
            Self::Count { .. }
                | Self::Dimen { .. }
                | Self::BoxDimensionAssignment { .. }
                | Self::Skip { .. }
                | Self::Muskip { .. }
                | Self::Toks { .. }
                | Self::IntParam { .. }
                | Self::DimenParam { .. }
                | Self::TokParam { .. }
                | Self::GlueParam { .. }
                | Self::CodeTable { .. }
                | Self::PdfFontCode { .. }
                | Self::PdfNoLigatures { .. }
                | Self::FontDimen { .. }
                | Self::FontInteger { .. }
                | Self::FontDefinition { .. }
                | Self::GeneratedFontDefinition { .. }
                | Self::InputStream { .. }
                | Self::Arithmetic { .. }
                | Self::InvalidArithmeticTarget { .. }
                | Self::CharacterDefinition { .. }
                | Self::RegisterDefinition { .. }
                | Self::ParagraphShape { .. }
                | Self::PenaltyArray { .. }
                | Self::FontSelect { .. }
                | Self::MathFamily { .. }
                | Self::SetBox { .. }
                | Self::PrevDepth { .. }
                | Self::SpaceFactor { .. }
                | Self::PrevGraf { .. }
                | Self::PageDimension { .. }
                | Self::PageInteger { .. }
                | Self::HyphenationData { .. }
                | Self::SetInteractionMode(..)
        )
    }

    pub(in crate::main_control) fn main_loop_character(&self) -> Option<char> {
        match *self {
            Self::Character {
                ch,
                cat: Catcode::Letter | Catcode::Other,
                ..
            } => Some(ch),
            Self::CharacterCode { value, .. } => u32::try_from(value).ok().and_then(char::from_u32),
            _ => None,
        }
    }
}

/// A completed assignable quantity selector.  It is intentionally a semantic
/// selector, never a delivered command or a raw input handle.
#[derive(Clone, Copy, Debug)]
pub(in crate::main_control) enum ArithmeticTarget {
    IntegerRegister(u16),
    DimensionRegister(u16),
    GlueRegister { index: u16, mu: bool },
    IntegerParameter(u16),
    DimensionParameter(u16),
    GlueParameter { index: u16, mu: bool },
}

#[derive(Clone, Copy, Debug)]
pub(in crate::main_control) enum ArithmeticOperand {
    Integer(i32),
    Dimension(Scaled),
    Glue(GlueSpec),
}
