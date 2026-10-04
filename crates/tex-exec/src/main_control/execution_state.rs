//! Actual execution, output and driver owners, disjoint from reader loans.
use super::*;
#[derive(Debug)]
pub(super) struct ExecutionState<G> {
    /// Operational memo service owned by the execution/session layer.
    pub(super) pure_memo: Arc<std::sync::Mutex<tex_state::PureMemoRuntime>>,
    pub(super) pure_memo_initialized: bool,
    /// Cold-bound direct handles into the immutable primitive registry.
    /// These are process-local accelerators, not TeX state or format data.
    pub(super) primitive_registry_len: Option<usize>,
    pub(super) pdf_ignore_depth: Option<tex_state::PrimitiveHandle<G>>,
    /// Host output capability for shipout traversal. `None` preserves the
    /// profile default: TeX/e-TeX emit DVI, while pdfTeX defers pages to its
    /// PDF driver. Virtual multi-output sessions set this explicitly.
    pub(super) emit_dvi_override: Option<bool>,
    pub(super) modes: ModeNest,
    /// Maximum typed §273/§275 checked pre-push save-stack projection. Runtime
    /// diagnostics do not participate in rollback, identity, or formats.
    pub(super) max_save_stack: usize,
    pub(super) next_alignment_identity: u64,
    pub(super) active_alignment: Option<ActiveReplayAlignment<G>>,
    pub(super) boxes: ReplayBoxes<G>,
    pub(super) active_discretionaries: Vec<ActiveDiscretionary>,
    /// TeX82 §1174's `saved(-2)` branch count for each live `\mathchoice`,
    /// outermost first. e-TeX [49.1292] observes it through `\showgroups`.
    pub(super) active_math_choices: Vec<usize>,
    /// TeX82 §1153's `saved(0)` field destinations for live `math_group`
    /// levels, outermost first. The save stack owns scope restoration; this
    /// executor-side vector owns only the typed parent-list destination.
    pub(super) active_math_fields: Vec<ActiveMathFieldTarget>,
    /// e-TeX [48.1191]'s saved delimiter identity for each live
    /// `math_left_group`, outermost first. `true` denotes `\middle`.
    pub(super) active_math_left_boundaries: Vec<bool>,
    /// Live `math_shift_group` openers, outermost first.
    pub(super) active_math_shifts: Vec<MathShiftContext>,
    /// Physical glue-store identity plus the canonical pointer source of the
    /// last skip-register definition. The second component is `None` when
    /// scanning allocated a fresh TeX glue node that Umber subsequently
    /// hash-consed with an equal existing node.
    pub(super) skip_pointer_sources: Vec<GluePointerSource<G>>,
    /// Mu-glue counterpart of [`Self::skip_pointer_sources`]. e-TeX's
    /// `\gluetomu` and `\mutoglue` conversions retain the source pointer, so
    /// the two register banks need the same identity accounting.
    pub(super) muskip_pointer_sources: Vec<GluePointerSource<G>>,
    /// True while `main_control` is parked at TeX82 §1034's
    /// `main_loop_lookahead` rather than at §1030's `big_switch`.
    ///
    /// TeX's inner character loop appends a character and then fetches the
    /// next command from §1038's lookahead, which starts with a bare
    /// `get_next`; only `big_switch` uses `get_x_token`. Umber executes one
    /// command per operation, so the label it would have jumped to has to
    /// be carried across steps explicitly.
    pub(super) main_loop_active: bool,
    /// TeX82's temporary `set_box_allowed:=false` ownership while §1270
    /// executes assignments after a display alignment or an accent.
    pub(super) set_box_forbidden_depth: u8,
    /// The last mode printed by TeX82 §1030's `show_cur_cmd_chr`.
    ///
    /// Zero is not a TeX mode, so `None` is WEB's initial `shown_mode=0`.
    /// This is diagnostic runtime state: it survives ordinary command steps
    /// but participates in an atomic step rollback.
    pub(super) shown_mode: Option<Mode>,
    /// False until TeX82 §1030's `main_control` prologue has run.
    ///
    /// `main_control` is entered once per job and opens with
    /// `if every_job<>null then begin_token_list(every_job,every_job_text)`,
    /// before `big_switch` fetches anything. Umber executes one command per
    /// step, so the prologue is carried as a one-shot on the first step rather
    /// than by a distinct entry call every driver would have to remember.
    pub(super) main_control_entered: bool,
    /// tex.web's `init`/`tini` compile-time split as a session flag.
    ///
    /// tex.web builds INITEX and production TeX from the same source with
    /// `init`-guarded code removed from the latter, so §1252's `\patterns`
    /// and §1335's `\dump` have entirely different behavior in the two
    /// binaries. Umber has one binary, so the distinction is the session's:
    /// [`MainControl::tex82_initex`] builds an INITEX session and
    /// every other constructor a production one.
    pub(super) initex: bool,
    /// Set when this session was started from a dumped format, so §61/§536's
    /// banner can name it (`(preloaded format=…)`) the way a real `-fmt` run
    /// does. `None` leaves the banner to `initex` above. Framing-only: no
    /// execution decision reads it.
    pub(super) preloaded_format: Option<crate::job::PreloadedFormat>,
    /// Engine identity used for §61/§536 startup framing and canonical
    /// compiled semantics that remain active across a loaded older format.
    ///
    /// Most jobs use the command profile's dialect. Reference-backed jobs may
    /// deliberately exercise an older semantic profile in a newer engine
    /// binary, so backend banner identity is an independent typed fact.
    pub(super) engine_binary: Option<crate::job::EngineBinaryIdentity>,
    /// True after TeX82 §1335 has successfully completed an INITEX `\dump`.
    ///
    /// This is a committed termination receipt for the host boundary; it is
    /// not format serialization itself.
    pub(super) dumped_format: Option<crate::job::FormatDumpReceipt>,
    /// TeX82 §331's `**` line, retained for §313's pseudoprint of it.
    ///
    /// §331 opens the base terminal level over the line naming the job's root
    /// file and never closes it, so the line stays pseudoprintable in
    /// `buffer` for the whole run -- §360's `*` prompt reads over it, and a
    /// failed read (§71) leaves it untouched. Umber retires the base level as
    /// soon as `scan_startup_file_name` has consumed it, so the line has to be
    /// kept here for [`crate::job::prompt_for_more_input`] to render.
    /// Empty when no startup line was scanned, which is §313's own display for
    /// a base level whose consumed and pending text are both empty.
    pub(super) startup_terminal_line: String,
    /// Whether §360's most recently acquired terminal line was empty. `None`
    /// means no `*`-prompt line has been acquired yet, so the startup line's
    /// emptiness supplies the first value.
    pub(super) terminal_line_was_empty: Option<bool>,
    /// Whether root exhaustion is TeX82 §360's missing-`\end` path or an
    /// explicit host-owned fragment boundary.
    pub(super) root_completion: RootCompletionPolicy,
    /// Scalar identity of the one host-selected root main input file.
    ///
    /// This is editor/restart and startup-framing identity, not checkpoint
    /// retention policy. Boundary formation freezes the independently carried
    /// source role; token provenance and macro names never participate.
    pub(super) root_main_source: Option<tex_state::SourceId>,
    /// The ordered publication transaction produced by
    /// `fire_pending_page_output` after the contributing step's own records.
    /// It first receives any earlier named-list pushes still owned by that
    /// direct step, then TeX82 §§1025/323's `output_text` push. Every step
    /// drains it before the following step can deliver the routine's
    /// scanner-owned opening brace.
    pub(super) page_output_observations: Option<ObservationBuffer>,
    /// Live-store boundaries used to close the typed receipt before the
    /// direct operation commits. Absent for ordinary execution.
    pub(super) operation_receipt_start: Option<OperationReceiptStart>,
    /// True when the most recent operation returned an error after committing
    /// its direct-operation roots. Observed drivers use this narrow query to
    /// publish the committed receipt even though the typed result is `Err`.
    pub(super) error_operation_committed: bool,
    /// Detached DVI receipts whose artifact commits have survived an entire
    /// direct operation. This replay state is published only after its
    /// corresponding World artifact/effect roots commit.
    pub(super) prepared_dvi_pages: PreparedDviPages,
    pub(super) prepared_shipout: Option<PreparedShipout>,
    pub(super) prepared_pdf_form: Option<tex_state::PdfFormRecord<G>>,
    /// Actual terminal notices, moved once after command admission ends.
    pub(super) pending_job_end: Option<(bool, Vec<tex_command::IncompleteCondition>)>,
    /// A detached page has committed, but a live mode still owns page-region
    /// coordinates. Succession waits until that mode closure is consumed into
    /// PageBuilder state; it never copies or scans the mode payload.
    pub(super) page_region_succession_pending: bool,
    /// Named safe boundaries committed by the last direct operation. The
    /// host drains these only after `advance` has committed, so a resource
    /// suspension never leaks a checkpoint from its rolled-back operation.
    /// Demand is installed by the runner before execution. A paragraph cut
    /// either consumes it immediately or is permanently skipped.
    pub(super) paragraph_checkpoint_demand: Option<bool>,
    pub(super) paragraph_checkpoint_cut: bool,
    /// The sole job-start restart capability. Restored controls do not regain
    /// it, and successful initial capture consumes it permanently.
    pub(super) job_start_eligibility: Option<crate::checkpoint::CheckpointEligibility>,
    /// Ordered named-boundary intents waiting for command-owned scanner,
    /// macro, resource, and structural continuations to become quiescent.
    /// Source site of the most recent typed resource suspension. This is
    /// retained outside snapshots so a host protocol no-progress invariant
    /// can still identify the command whose retry failed to advance.
    pub(super) pending_resource_site: Option<OriginId>,
    /// A command failed or suspended mid-flight. Owners keep whatever the
    /// command had already written, so only a full checkpoint restore clears
    /// this latch; nothing here unwinds a partial command.
    pub(super) halted: bool,
    /// Set exactly when the runner receives End. No later delivery is legal.
    pub(super) ended: bool,
    /// TeX82 §76's `history=fatal_error_stop`, carrying §93/§94/§95's payload.
    ///
    /// `succumb` ends the job through §81's `jump_out`, which a library engine
    /// cannot spell as leaving the process. This latch is the canonical
    /// equivalent: once it is set the session is terminal, every further
    /// operation reports [`MainControlStep::End`] without delivering a
    /// command, and the host reads the cause from [`MainControl::fatal_error`].
    pub(super) fatal: Option<FatalError>,
    /// Source evidence retained when a fatal crossed a diagnostic capture
    /// seam. Diagnostic session drivers surface this exact location to their
    /// caller; complete-job drivers retain TeX's terminal completion semantics.
    pub(super) captured_fatal_origin: Option<(
        DiagnosticSite,
        Option<crate::FrozenDiagnosticOrigin>,
        Option<crate::FrozenDiagnosticContext>,
    )>,
    /// First recoverable error's bounded stack evidence. Trace-only
    /// diagnostics do not populate it.
    pub(super) first_causal_context: Option<crate::FrozenDiagnosticContext>,
    /// First committed recoverable report for explicit causal diagnostics.
    /// This runtime evidence is not part of command snapshots, convergence
    /// identity, semantic hashes, or output event streams.
    pub(super) first_recoverable_diagnostic: Option<Box<crate::FirstRecoverableDiagnostic>>,
    /// tex.web's job-framing state: see [`crate::job`] and
    /// `docs/job_framing.md`.
    pub(super) job: crate::job::JobFraming,
    /// Effect-record boundary immediately before §1335 final-cleanup
    /// framing, extended through later pdfTeX navigation warnings when they
    /// exist. Drivers can project a root body without deleting framing.
    pub(super) job_body_effect_end: Option<tex_state::EffectPos>,
    /// Guards the host-owned pdfTeX close-files pass against repeated
    /// `EngineSession::finish` calls after termination.
    pub(super) pdf_navigation_finalized: bool,
    /// Operational accounting only; snapshots and durable checkpoints never
    /// observe it.
    pub(super) advance_telemetry: AdvanceTelemetry,
    /// Monotonic evidence for bounded semantic episode admission and return.
    /// Like command fuel, this is operational and rollback never refunds it.
    pub(super) episode_telemetry: crate::EpisodeTelemetry,
}

impl<G> ExecutionState<G> {
    pub(super) fn with_modes(modes: ModeNest) -> Self {
        Self {
            pure_memo: Arc::default(),
            pure_memo_initialized: false,
            primitive_registry_len: None,
            pdf_ignore_depth: None,
            emit_dvi_override: None,
            modes,
            max_save_stack: 0,
            next_alignment_identity: 0,
            active_alignment: None,
            boxes: ReplayBoxes::default(),
            active_discretionaries: Vec::new(),
            active_math_choices: Vec::new(),
            active_math_fields: Vec::new(),
            active_math_left_boundaries: Vec::new(),
            active_math_shifts: Vec::new(),
            skip_pointer_sources: Vec::new(),
            muskip_pointer_sources: Vec::new(),
            main_loop_active: false,
            set_box_forbidden_depth: 0,
            shown_mode: None,
            main_control_entered: false,
            initex: false,
            preloaded_format: None,
            engine_binary: None,
            dumped_format: None,
            startup_terminal_line: String::new(),
            terminal_line_was_empty: None,
            root_completion: RootCompletionPolicy::default(),
            root_main_source: None,
            page_output_observations: None,
            operation_receipt_start: None,
            error_operation_committed: false,
            prepared_dvi_pages: PreparedDviPages::default(),
            prepared_shipout: None,
            prepared_pdf_form: None,
            pending_job_end: None,
            page_region_succession_pending: false,
            paragraph_checkpoint_demand: None,
            paragraph_checkpoint_cut: false,
            job_start_eligibility: Some(crate::checkpoint::CheckpointEligibility::job_start()),
            pending_resource_site: None,
            halted: false,
            ended: false,
            fatal: None,
            captured_fatal_origin: None,
            first_causal_context: None,
            first_recoverable_diagnostic: None,
            job: crate::job::JobFraming::default(),
            job_body_effect_end: None,
            pdf_navigation_finalized: false,
            advance_telemetry: AdvanceTelemetry::default(),
            episode_telemetry: crate::EpisodeTelemetry::default(),
        }
    }
}
