use crate::env::AssignmentScope;
use crate::env::banks::IntParam;
use crate::interner::InternerBudget;
use crate::world::{EffectRecord, PrintSink};
use crate::{GroupKind, Universe};

fn with_test_universe<R>(
    use_universe: impl for<'id> FnOnce(&mut Universe<crate::GenerationBrand<'id>>) -> R,
) -> R {
    let budget = InternerBudget::new(16, 16, 256).expect("budget");
    crate::with_universe(budget, use_universe).expect("fresh universe")
}

fn routed_log<G>(universe: &Universe<G>) -> String {
    universe
        .world()
        .effect_records()
        .iter()
        .filter_map(|record| match record {
            EffectRecord::StreamWrite {
                sink: PrintSink::Log,
                text,
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn enable_tracing<G>(universe: &mut Universe<G>) {
    universe
        .assign_int_param(IntParam::TRACING_GROUPS, 1, AssignmentScope::Global)
        .expect("enable tracinggroups");
}

fn trace_enter<G>(universe: &mut Universe<G>, kind: GroupKind, level: u32, line: u32) {
    let mut effects = crate::diagnostic::DiagnosticEffects::new();
    universe
        .command_context()
        .expect("group trace admission")
        .trace_group_enter(&mut effects, kind, level, line);
    universe.world_mut().publish_diagnostic_effects(effects);
}

fn trace_leave<G>(universe: &mut Universe<G>, kind: GroupKind, level: u32, line: u32) {
    let mut effects = crate::diagnostic::DiagnosticEffects::new();
    universe
        .command_context()
        .expect("group trace admission")
        .trace_group_leave(&mut effects, kind, level, line);
    universe.world_mut().publish_diagnostic_effects(effects);
}

#[test]
fn tracinggroups_zero_emits_nothing() {
    with_test_universe(|universe| {
        trace_enter(universe, GroupKind::Simple, 1, 5);
        assert_eq!(routed_log(universe), "");
    });
}

#[test]
fn entering_groups_reports_kind_depth_and_source_line() {
    with_test_universe(|universe| {
        enable_tracing(universe);
        trace_enter(universe, GroupKind::Simple, 1, 4);
        trace_enter(universe, GroupKind::AdjustedHBox, 2, 5);
        assert_eq!(
            routed_log(universe),
            "{entering simple group (level 1) at line 4}\n\
             {entering adjusted hbox group (level 2) at line 5}\n"
        );
    });
}

#[test]
fn zero_source_line_is_omitted() {
    with_test_universe(|universe| {
        enable_tracing(universe);
        trace_enter(universe, GroupKind::Simple, 1, 0);
        assert_eq!(routed_log(universe), "{entering simple group (level 1)}\n");
    });
}

#[test]
fn leaving_a_group_reports_the_closed_frame() {
    with_test_universe(|universe| {
        enable_tracing(universe);
        let frame = universe
            .begin_group(GroupKind::VBox, 313)
            .expect("begin vbox group");
        trace_enter(universe, frame.kind(), 1, frame.entered_line());
        let before_leave = routed_log(universe).len();
        let frame = universe
            .end_group(GroupKind::VBox)
            .expect("end vbox group")
            .frame();
        trace_leave(universe, frame.kind(), 1, frame.entered_line());

        assert_eq!(
            &routed_log(universe)[before_leave..],
            "{leaving vbox group (level 1) entered at line 313}\n"
        );
    });
}

#[test]
fn restored_tracinggroups_value_controls_the_exit_trace() {
    with_test_universe(|universe| {
        enable_tracing(universe);
        universe
            .begin_group(GroupKind::SemiSimple, 13)
            .expect("begin group");
        universe
            .assign_int_param(IntParam::TRACING_GROUPS, 0, AssignmentScope::Local)
            .expect("locally disable tracinggroups");
        let frame = universe
            .end_group(GroupKind::SemiSimple)
            .expect("end group")
            .frame();
        trace_leave(universe, frame.kind(), 1, frame.entered_line());

        assert_eq!(
            routed_log(universe),
            "{leaving semi simple group (level 1) entered at line 13}\n"
        );
    });
}
