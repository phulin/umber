use super::{
    AlignmentDeliveryState, AlignmentIdentity, PREAMBLE_ALIGN_STATE, TOP_LEVEL_ALIGN_STATE,
};

#[test]
fn alignment_lifecycle_restores_the_running_outer_brace_state() {
    let mut state = AlignmentDeliveryState::<()>::default();
    state.align_state = TOP_LEVEL_ALIGN_STATE + 7;
    let alignment = AlignmentIdentity::new(1);
    state.begin_alignment(alignment);
    assert_eq!(state.align_state, PREAMBLE_ALIGN_STATE);
    state.finish_alignment(alignment).expect("finish");
    assert_eq!(state.align_state, TOP_LEVEL_ALIGN_STATE + 7);
}

#[test]
fn nested_alignment_suspension_restores_the_exact_outer_identity() {
    let mut state = AlignmentDeliveryState::<()>::default();
    let outer = AlignmentIdentity::new(1);
    let inner = AlignmentIdentity::new(2);
    state.begin_alignment(outer);
    state.suspend_alignment(outer).expect("suspend outer");
    state.begin_alignment(inner);
    state.finish_alignment(inner).expect("finish inner");
    state.resume_alignment(outer).expect("resume outer");
    assert_eq!(state.active_alignment, Some(outer));
    state.finish_alignment(outer).expect("finish outer");
    assert_eq!(state.align_state, TOP_LEVEL_ALIGN_STATE);
}
