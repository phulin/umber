use super::{ControlSequenceKind, Interner};
use crate::interner::{Symbol, SymbolId};

fn intern(interner: &mut Interner, name: &str) -> Symbol {
    interner
        .intern(name)
        .expect("test interner should not reach symbol capacity")
        .symbol()
}

#[test]
fn intern_is_idempotent() {
    let mut interner = Interner::new();

    let first = intern(&mut interner, "count");
    let second = intern(&mut interner, "count");

    assert_eq!(first, second);
    assert_eq!(interner.len(), 1);
}

#[test]
fn semantic_identity_is_cached_once_per_control_sequence_slot() {
    // TeX82 §§222, 259 give one canonical name/kind identity to a control
    // sequence. Large macro token lists may repeat that token millions of
    // times; freezing them must reuse the interner-owned identity instead of
    // allocating and hashing the spelling once per occurrence.
    let mut interner = Interner::new();
    let symbol = intern(&mut interner, "large_list_control_sequence");
    let (atom, cached) = interner
        .semantic_atom_identity(symbol)
        .expect("live symbol has cached semantic projections");

    for _ in 0..10_000 {
        assert_eq!(
            interner.semantic_atom_identity(symbol),
            Some((atom, cached))
        );
    }
    assert_eq!(interner.semantic_identities, [cached]);
}

#[test]
fn resolve_round_trips_ascii_and_non_ascii() {
    let mut interner = Interner::new();

    let ascii = intern(&mut interner, "par");
    let non_ascii = intern(&mut interner, "é漢字🙂");

    assert_eq!(interner.resolve(ascii), "par");
    assert_eq!(interner.resolve(non_ascii), "é漢字🙂");
}

#[test]
fn active_character_and_same_spelling_named_sequence_are_distinct() {
    let mut interner = Interner::new();

    let named = intern(&mut interner, "~");
    let active = interner.intern_active('~').expect("active symbol").symbol();

    assert_ne!(named, active);
    assert_eq!(interner.get("~").map(SymbolId::symbol), Some(named));
    assert_eq!(interner.get_active('~').map(SymbolId::symbol), Some(active));
    assert_eq!(interner.resolve(named), "~");
    assert_eq!(interner.resolve(active), "~");
    assert_eq!(interner.kind(named), ControlSequenceKind::SingleCharacter);
    assert_eq!(interner.kind(active), ControlSequenceKind::ActiveCharacter);
}

#[test]
fn multiletter_count_is_the_tex82_hash_namespace() {
    let mut interner = Interner::new();

    intern(&mut interner, "");
    intern(&mut interner, "x");
    interner.intern_active('x').expect("active symbol");
    intern(&mut interner, "xx");
    intern(&mut interner, "multiletter");
    interner
        .intern_internal("inaccessible")
        .expect("internal symbol");

    assert_eq!(interner.len(), 6);
    assert_eq!(interner.multiletter_len(), 2);
}

#[test]
fn hash_occupancy_excludes_one_letter_names_and_survives_internal_aliasing() {
    let mut interner = Interner::new();

    // TeX82 §§356/372 route a one-character spelling to its fixed eqtb slot,
    // outside §259's hash. The ordinary multiletter `nullfont` entry remains
    // counted after §222 installs its frozen alias.
    let x = interner.intern_hash("x").expect("one-letter hash name");
    let nullfont = interner
        .intern_hash("nullfont")
        .expect("ordinary primitive");
    interner
        .intern_internal("nullfont")
        .expect("fixed internal alias");
    let inaccessible = interner
        .intern_internal("inaccessible")
        .expect("unhashed fixed alias");

    assert!(!interner.is_hash_entry(x.symbol()));
    assert!(interner.is_hash_entry(nullfont.symbol()));
    assert!(!interner.is_hash_entry(inaccessible.symbol()));
    assert_eq!(interner.multiletter_len(), 1);
}

#[test]
fn rollback_rebuild_preserves_control_sequence_namespace() {
    let mut interner = Interner::new();
    let named = intern(&mut interner, "~");
    let mark = interner.watermark();
    let discarded_active = interner.intern_active('~').expect("active symbol").symbol();

    interner.truncate_to(mark);
    assert_eq!(interner.get("~").map(SymbolId::symbol), Some(named));
    assert_eq!(interner.get_active('~'), None);

    let active = interner
        .intern_active('~')
        .expect("reintern active symbol")
        .symbol();
    assert_eq!(active, discarded_active);
    assert_ne!(active, named);
}

#[test]
fn concurrent_interners_share_stable_global_name_identity() {
    let threads = (0..16)
        .map(|_| {
            std::thread::spawn(|| {
                let mut interner = Interner::new();
                ["relax", "hrule", "vbox", "setbox", "looseness"]
                    .map(|name| intern(&mut interner, name))
            })
        })
        .collect::<Vec<_>>();
    let symbols = threads
        .into_iter()
        .map(|thread| thread.join().expect("interner thread"))
        .collect::<Vec<_>>();

    assert!(symbols.windows(2).all(|pair| pair[0] == pair[1]));
}
