use core::cell::Cell;
use std::rc::Rc;

use super::{
    CommandArenaCursors, CommandSnapshotCursor, CommandStackCursors, CommandStateSnapshot,
    CommandSummary, CommandSummaryError,
};

struct Brand;

#[derive(Debug)]
struct CountingOwner {
    clones: Rc<Cell<u32>>,
}

impl Clone for CountingOwner {
    fn clone(&self) -> Self {
        self.clones.set(self.clones.get() + 1);
        Self {
            clones: Rc::clone(&self.clones),
        }
    }
}

fn cursor(seed: u32) -> CommandSnapshotCursor {
    CommandSnapshotCursor::new(
        seed,
        CommandArenaCursors::new(seed + 1, seed + 2, seed + 3, seed + 4, seed + 5),
        CommandStackCursors::new(
            seed + 6,
            seed + 7,
            seed + 8,
            seed + 9,
            seed + 10,
            seed + 11,
            seed + 12,
        ),
    )
}

#[test]
fn snapshot_clone_retains_one_coarse_owner_and_copies_only_cursors() {
    let clones = Rc::new(Cell::new(0));
    let snapshot = CommandStateSnapshot::<Brand, _>::new(
        CountingOwner {
            clones: Rc::clone(&clones),
        },
        cursor(7),
    );

    let cloned = snapshot.clone();

    assert_eq!(clones.get(), 1);
    assert_eq!(cloned.cursor(), cursor(7));
    assert_eq!(cloned.cursor().command_journal(), 7);
    assert_eq!(cloned.cursor().arenas().attempt_rows(), 12);
    assert_eq!(cloned.cursor().stacks().framing_event_count(), 19);
}

#[test]
fn summary_is_a_coarse_owner_plus_fixed_restart_coordinates() {
    let clones = Rc::new(Cell::new(0));
    let summary = CommandSummary::<Brand, _>::new(
        CountingOwner {
            clones: Rc::clone(&clones),
        },
        cursor(13),
        0xfeed_beef,
        Some(1_024),
    );
    let cloned = summary.clone();
    let (_owner, restored, profile, anchor) = cloned.into_parts();

    assert_eq!(clones.get(), 1);
    assert_eq!(restored, cursor(13));
    assert_eq!(profile, 0xfeed_beef);
    assert_eq!(anchor, Some(1_024));
}

#[test]
fn cursor_components_are_copy_only_scalar_coordinates() {
    fn assert_copy<T: Copy>() {}

    assert_copy::<CommandArenaCursors>();
    assert_copy::<CommandStackCursors>();
    assert_copy::<CommandSnapshotCursor>();

    let mark = cursor(1);
    assert_eq!(mark.arenas().input_rows(), 2);
    assert_eq!(mark.arenas().input_words(), 3);
    assert_eq!(mark.arenas().parameter_words(), 4);
    assert_eq!(mark.arenas().builder_words(), 5);
    assert_eq!(mark.stacks().input_depth(), 7);
    assert_eq!(mark.stacks().parameter_depth(), 8);
    assert_eq!(mark.stacks().condition_depth(), 9);
    assert_eq!(mark.stacks().alignment_depth(), 10);
    assert_eq!(mark.stacks().replay_depth(), 11);
    assert_eq!(mark.stacks().diagnostic_count(), 12);
}

#[test]
fn summary_rejection_names_suspended_attempts_separately() {
    assert_eq!(
        CommandSummaryError::AttemptSuspended.to_string(),
        "the command attempt is owned by a suspension"
    );
    assert_eq!(
        CommandSummaryError::ResourceSuspension.to_string(),
        "a command resource request is pending"
    );
}
