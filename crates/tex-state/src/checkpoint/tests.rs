use super::{BoundedStateMark, GenerationCheckpoint, RestoreTarget, prepare_restore};

#[derive(Debug, Eq, PartialEq)]
struct Owner(&'static str);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Mark {
    journal: u32,
    arena: u32,
}

#[derive(Debug, Eq, PartialEq)]
struct Target {
    valid: bool,
    state: i32,
    roots: i32,
    suffix: i32,
    acquired: Option<Owner>,
    phases: Vec<&'static str>,
}

impl RestoreTarget<Owner, Mark> for Target {
    type Error = &'static str;
    type Output = Owner;

    fn validate_restore(&self, owner: &Owner, mark: &Mark) -> Result<(), Self::Error> {
        if self.valid && owner.0 == "generation" && mark.journal <= 7 && mark.arena <= 11 {
            Ok(())
        } else {
            Err("invalid restore coordinate")
        }
    }

    fn acquire_target_owner(&mut self, owner: Owner) {
        self.phases.push("owner");
        self.acquired = Some(owner);
    }

    fn restore_dense_state(&mut self, mark: &Mark) {
        assert!(self.acquired.is_some());
        self.phases.push("dense");
        self.state = mark.journal as i32;
    }

    fn transfer_roots(&mut self, mark: &Mark) {
        assert!(self.acquired.is_some());
        self.phases.push("roots");
        self.roots = mark.arena as i32;
    }

    fn truncate_suffixes(&mut self, mark: &Mark) {
        assert_eq!(self.roots, mark.arena as i32);
        self.phases.push("truncate");
        self.suffix = mark.arena as i32;
    }

    fn release_replaced_owners(&mut self) -> Self::Output {
        self.phases.push("release");
        self.acquired.take().expect("owner acquired before release")
    }
}

fn target(valid: bool) -> Target {
    Target {
        valid,
        state: 91,
        roots: 92,
        suffix: 93,
        acquired: None,
        phases: Vec::new(),
    }
}

#[test]
fn malformed_restore_is_mutation_free() {
    let target = target(false);
    let before = Target {
        valid: target.valid,
        state: target.state,
        roots: target.roots,
        suffix: target.suffix,
        acquired: None,
        phases: Vec::new(),
    };
    let checkpoint = GenerationCheckpoint::new(
        Owner("generation"),
        Mark {
            journal: 7,
            arena: 11,
        },
    );

    assert!(prepare_restore(&target, checkpoint).is_err());
    assert_eq!(target, before);
}

#[test]
fn restore_order_is_owner_then_state_then_roots_then_truncation_then_release() {
    let mut target = target(true);
    let checkpoint = GenerationCheckpoint::new(
        Owner("generation"),
        Mark {
            journal: 7,
            arena: 11,
        },
    );

    let plan = prepare_restore(&target, checkpoint).expect("valid plan");
    assert!(target.phases.is_empty(), "planning must not mutate");
    assert_eq!(plan.apply(&mut target), Owner("generation"));
    assert_eq!(
        target.phases,
        ["owner", "dense", "roots", "truncate", "release"]
    );
    assert_eq!((target.state, target.roots, target.suffix), (7, 11, 11));
}

#[test]
fn checkpoints_move_a_nonclone_owner_beside_only_copyable_marks() {
    let mark = BoundedStateMark::new(1_u32, 2_u32, 3_u32, 4_u32);
    let checkpoint = GenerationCheckpoint::new(Owner("generation"), mark);

    assert_eq!(*checkpoint.mark().journal(), 1);
    assert_eq!(*checkpoint.mark().durable(), 2);
    assert_eq!(*checkpoint.mark().page(), 3);
    assert_eq!(*checkpoint.mark().input(), 4);
}
