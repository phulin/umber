//! Concrete leader-glue scanner result.
use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) enum LeaderGlueResult {
    Payload {
        kind: GlueKind,
        payload: LeaderPayload,
    },
    Register {
        kind: GlueKind,
        index: u16,
        copy: bool,
    },
}
