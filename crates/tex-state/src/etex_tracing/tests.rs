use crate::env::banks::IntParam;
use crate::world::{EffectRecord, PrintSink};
use crate::{GroupKind, Universe};

/// Concatenates the routed text a diagnostic emitted, per sink; mirrors
/// `crate::diagnostic::tests::routed`, which is not visible from a sibling
/// module's own `#[cfg(test)]` unit.
fn routed_log(universe: &Universe) -> String {
    let mut text = String::new();
    for record in universe.world().effect_records() {
        if let EffectRecord::StreamWrite {
            sink: PrintSink::Log,
            text: chunk,
        } = record
        {
            text.push_str(chunk);
        }
    }
    text
}

fn tracing_universe() -> Universe {
    let mut universe = Universe::new();
    universe.set_int_param(IntParam::TRACING_GROUPS, 1);
    universe
}

#[test]
fn tracinggroups_zero_emits_nothing() {
    let mut universe = Universe::new();
    universe.enter_group_with_kind_at_line(GroupKind::Simple, 5);
    assert_eq!(routed_log(&universe), "");
}

#[test]
fn entering_a_group_reports_the_new_depth_and_source_line() {
    let mut universe = tracing_universe();
    universe.enter_group_with_kind_at_line(GroupKind::VBox, 313);
    assert_eq!(
        routed_log(&universe),
        "{entering vbox group (level 1) at line 313}\n"
    );
}

#[test]
fn entering_a_group_at_line_zero_omits_the_line_clause() {
    let mut universe = tracing_universe();
    universe.enter_group_with_kind(GroupKind::Simple);
    assert_eq!(routed_log(&universe), "{entering simple group (level 1)}\n");
}

#[test]
fn nested_groups_number_levels_by_open_depth() {
    let mut universe = tracing_universe();
    universe.enter_group_with_kind_at_line(GroupKind::Simple, 4);
    universe.enter_group_with_kind_at_line(GroupKind::AdjustedHBox, 5);
    assert_eq!(
        routed_log(&universe),
        "{entering simple group (level 1) at line 4}\n\
         {entering adjusted hbox group (level 2) at line 5}\n"
    );
}

#[test]
fn leaving_a_group_reports_its_own_level_and_entry_line() {
    let mut universe = tracing_universe();
    universe.enter_group_with_kind_at_line(GroupKind::Simple, 315);
    let before_leave = routed_log(&universe).len();
    universe
        .leave_group_with_kind(GroupKind::Simple)
        .expect("matching group kind");
    let leave_text = &routed_log(&universe)[before_leave..];
    assert_eq!(
        leave_text,
        "{leaving simple group (level 1) entered at line 315}\n"
    );
}

#[test]
fn leaving_a_nested_group_keeps_the_outer_frames_own_line() {
    let mut universe = tracing_universe();
    universe.enter_group_with_kind_at_line(GroupKind::VBox, 313);
    universe.enter_group_with_kind_at_line(GroupKind::AdjustedHBox, 314);
    let before_leave = routed_log(&universe).len();
    universe
        .leave_group_with_kind(GroupKind::AdjustedHBox)
        .expect("matching group kind");
    let leave_text = &routed_log(&universe)[before_leave..];
    assert_eq!(
        leave_text,
        "{leaving adjusted hbox group (level 2) entered at line 314}\n"
    );
}

#[test]
fn untyped_leave_group_traces_the_live_top_frame() {
    let mut universe = tracing_universe();
    universe.enter_group_with_kind_at_line(GroupKind::Simple, 7);
    let before_leave = routed_log(&universe).len();
    let _ = universe.leave_group();
    let leave_text = &routed_log(&universe)[before_leave..];
    assert_eq!(
        leave_text,
        "{leaving simple group (level 1) entered at line 7}\n"
    );
}

#[test]
fn locally_enabling_tracinggroups_does_not_trace_its_own_group_exit() {
    let mut universe = Universe::new();
    universe.enter_group_with_kind_at_line(GroupKind::SemiSimple, 12);
    universe.set_int_param(IntParam::TRACING_GROUPS, 1);

    universe
        .leave_group_with_kind(GroupKind::SemiSimple)
        .expect("matching group kind");

    // e-TeX [19.282] reaches `group_trace(true)` only after `unsave` has
    // restored the enclosing zero value of `\tracinggroups`.
    assert_eq!(routed_log(&universe), "");
}

#[test]
fn locally_disabling_tracinggroups_still_traces_its_group_exit() {
    let mut universe = tracing_universe();
    universe.enter_group_with_kind_at_line(GroupKind::SemiSimple, 13);
    universe.set_int_param(IntParam::TRACING_GROUPS, 0);
    let before_leave = routed_log(&universe).len();

    universe
        .leave_group_with_kind(GroupKind::SemiSimple)
        .expect("matching group kind");

    // The restored enclosing value enables the trace at [19.282]'s `done`.
    assert_eq!(
        &routed_log(&universe)[before_leave..],
        "{leaving semi simple group (level 1) entered at line 13}\n"
    );
}
