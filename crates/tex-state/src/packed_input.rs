//! Borrow-safe public seam for the canonical runtime input-frame layout.
//!
//! The wrapper exposes only copy cursor operations needed by `tex-command`.
//! Arena owners, coordinates, reservations, and admission remain private to
//! `tex-state`; the value derives no serialization and is not a checkpoint or
//! format handle.

pub use crate::hot_core::layout::{InputFrameFlags, InputFrameKind};

/// The canonical fixed-width live source/token cursor.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct InputFrame(crate::hot_core::layout::InputFrame);

impl InputFrame {
    #[must_use]
    pub fn source(identity: u64, source: crate::SourceId) -> Self {
        Self(crate::hot_core::layout::InputFrame::runtime(
            identity,
            u32::MAX,
            InputFrameKind::Source,
            InputFrameFlags::empty(),
            source.raw(),
        ))
    }

    #[must_use]
    pub fn tokens(identity: u64, len: u32, kind: InputFrameKind, flags: InputFrameFlags) -> Self {
        Self(crate::hot_core::layout::InputFrame::runtime(
            identity, len, kind, flags, 0,
        ))
    }

    #[must_use]
    pub const fn identity(self) -> u64 {
        self.0.runtime_identity()
    }

    #[must_use]
    pub const fn position(self) -> u32 {
        self.0.position()
    }

    #[must_use]
    pub const fn is_exhausted(self) -> bool {
        self.0.is_exhausted()
    }

    pub fn advance(&mut self) -> Option<u32> {
        let position = self.position();
        self.0.next_coordinate()?;
        Some(position)
    }

    pub fn add_flags(&mut self, flags: InputFrameFlags) {
        self.0.add_flags(flags);
    }

    pub fn extend_limit(&mut self, additional: u32) -> Option<()> {
        self.0.extend_limit(additional)
    }
}

const _: () = assert!(core::mem::size_of::<InputFrame>() == 40);

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::{InputFrame, InputFrameFlags, InputFrameKind};

    #[test]
    fn runtime_cursor_is_fixed_width_and_copy_only() {
        assert_eq!(size_of::<InputFrame>(), 40);
        let mut frame = InputFrame::tokens(
            41,
            2,
            InputFrameKind::BackedUp,
            InputFrameFlags::SUPPRESS_EXPANDABLE_CONTROL_SEQUENCE,
        );
        assert_eq!(frame.identity(), 41);
        assert_eq!(frame.advance(), Some(0));
        assert_eq!(frame.advance(), Some(1));
        assert_eq!(frame.advance(), None);
    }

    #[test]
    fn runtime_cursor_extends_without_replacing_its_identity() {
        let mut frame =
            InputFrame::tokens(9, 1, InputFrameKind::Inserted, InputFrameFlags::empty());
        assert_eq!(frame.advance(), Some(0));
        frame.extend_limit(2).expect("packed limit extends");
        frame.add_flags(InputFrameFlags::RETAIN_AT_END);
        assert_eq!(frame.identity(), 9);
        assert_eq!(frame.advance(), Some(1));
        assert_eq!(frame.advance(), Some(2));
        assert!(frame.is_exhausted());
    }
}
