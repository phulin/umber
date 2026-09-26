use super::*;

fn copied_font() -> LoadedFont {
    test_font().copied(vec![Scaled::from_raw(0); 7])
}

#[test]
fn identity_lookup_preserves_first_slot_without_deduplicating_copies() {
    let mut fonts = FontStore::new();
    let font = copied_font();
    let identity = font.realized_identity();
    let first = fonts.intern(font.clone()).expect("test font fits");
    let second = fonts.intern(font).expect("test font fits");
    assert_ne!(first, second, "copied fonts have independent mutable slots");
    assert_eq!(fonts.by_realized_identity(identity), Some(first));
    assert_eq!(
        fonts.by_realized_identity(fonts.get(NULL_FONT).realized_identity()),
        Some(NULL_FONT)
    );
}

#[test]
fn identity_lookup_drops_truncated_incarnations_and_follows_replacements() {
    let mut fonts = FontStore::new();
    let mark = fonts.watermark();
    let font = copied_font();
    let identity = font.realized_identity();
    let old = fonts.intern(font.clone()).expect("test font fits");
    let snapshot = fonts.clone();
    fonts.truncate_to(mark);
    assert_eq!(fonts.by_realized_identity(identity), None);
    assert_eq!(snapshot.by_realized_identity(identity), Some(old));
    let replacement = fonts.intern(font).expect("test font fits");
    assert_eq!(old.raw(), replacement.raw());
    assert_ne!(old, replacement);
    assert_eq!(fonts.by_realized_identity(identity), Some(replacement));
}

#[test]
fn identity_lookup_restores_rejected_tail_and_keeps_accepted_candidate() {
    let mut fonts = FontStore::new();
    let mark = fonts.watermark();
    let font = copied_font();
    let identity = font.realized_identity();
    let accepted = fonts.intern(font.clone()).expect("test font fits");
    let tail = fonts.begin_checkpoint_candidate(mark);
    assert_eq!(fonts.by_realized_identity(identity), None);
    let rejected = fonts.intern(font.clone()).expect("test font fits");
    assert_eq!(fonts.by_realized_identity(identity), Some(rejected));
    fonts.reject_checkpoint_candidate(mark, tail);
    assert_eq!(fonts.by_realized_identity(identity), Some(accepted));
    assert!(!fonts.contains(rejected));
    let tail = fonts.begin_checkpoint_candidate(mark);
    let replacement = fonts.intern(font).expect("test font fits");
    fonts.accept_checkpoint_candidate(tail);
    assert_eq!(fonts.by_realized_identity(identity), Some(replacement));
    assert!(!fonts.contains(accepted));
}

#[test]
fn retained_prefix_lookup_prefers_earliest_slot_and_excludes_parent_suffix() {
    let mut parent = FontStore::new();
    let font = copied_font();
    let identity = font.realized_identity();
    let first = parent.intern(font.clone()).expect("test font fits");
    let mark = parent.watermark();
    let suffix = test_font().expanded(50);
    let suffix_identity = suffix.realized_identity();
    parent.intern(suffix).expect("test font fits");
    let mut child = parent.fork_at(mark);
    assert_eq!(child.by_realized_identity(suffix_identity), None);
    child.intern(font.clone()).expect("test font fits");
    assert_eq!(child.by_realized_identity(identity), Some(first));
    let mut grandchild = child.fork_at(child.watermark());
    grandchild.intern(font).expect("test font fits");
    assert_eq!(grandchild.by_realized_identity(identity), Some(first));
}

#[test]
fn frozen_identity_index_rebuilds_first_slot_lookup() {
    let mut source = FontStore::new();
    let font = copied_font();
    let identity = font.realized_identity();
    source.intern(font.clone()).expect("test font fits");
    source.intern(font).expect("test font fits");
    let payloads = (0..source.len())
        .map(|raw| {
            source
                .get(source.id_at(raw as u32).expect("source slot is live"))
                .clone()
        })
        .collect();
    let restored = FontStore::from_frozen(
        payloads,
        vec![None; 3],
        vec![None; 3],
        &crate::interner::Interner::new(
            crate::interner::InternerBudget::new(16, 16, 256).expect("test interner budget"),
        ),
    )
    .expect("frozen font rows restore");
    assert_eq!(restored.by_realized_identity(identity), restored.id_at(1));
}
