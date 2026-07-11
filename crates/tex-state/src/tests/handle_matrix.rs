use crate::font::{FontMetrics, LoadedFont};
use crate::glue::{GlueSpec, Order};
use crate::ids::{NodeListId, TokenListId};
use crate::macro_store::MacroMeaning;
use crate::meaning::MeaningFlags;
use crate::node::{GlueKind, Node};
use crate::provenance::SyntheticOriginKind;
use crate::scaled::Scaled;
use crate::source_map::SourceDescriptor;
use crate::token::{Catcode, Token};
use crate::world::ContentHash;
use crate::{SourceId, Universe, World};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
enum HandleClass {
    Symbol,
    TokenList,
    MacroDefinition,
    Glue,
    Font,
    OriginList,
    ArenaOrigin,
    SourcePosition,
    EpochNodeList,
    SurvivorNodeList,
    WorldInputRecord,
}

const HANDLE_CLASSES: &[HandleClass] = &[
    HandleClass::Symbol,
    HandleClass::TokenList,
    HandleClass::MacroDefinition,
    HandleClass::Glue,
    HandleClass::Font,
    HandleClass::OriginList,
    HandleClass::ArenaOrigin,
    HandleClass::SourcePosition,
    HandleClass::EpochNodeList,
    HandleClass::SurvivorNodeList,
    HandleClass::WorldInputRecord,
];

#[test]
fn rollback_reallocate_matrix_rejects_every_stale_handle() {
    for &class in HANDLE_CLASSES {
        exercise_rollback_reallocate(class);
    }
}

#[test]
fn fork_matrix_preserves_inherited_handles_and_rejects_sibling_handles() {
    for &class in HANDLE_CLASSES {
        exercise_fork(class);
    }
}

#[test]
fn cross_universe_matrix_rejects_every_foreign_handle() {
    for &class in HANDLE_CLASSES {
        exercise_cross_universe(class);
    }
}

#[test]
fn page_ingress_validates_before_mutating() {
    let mut owner = Universe::new();
    let foreign = owner.intern_glue(glue(7));
    let foreign_node = glue_node(foreign);
    let mut universe = Universe::new();

    assert_panics(HandleClass::Glue, || {
        universe.append_page_contribution(foreign_node.clone())
    });
    assert_panics(HandleClass::Glue, || {
        universe.prepend_page_contribution(foreign_node.clone())
    });
    assert_panics(HandleClass::Glue, || {
        universe.push_current_page_node(foreign_node.clone())
    });
    assert!(universe.page_contributions().is_empty());
    assert!(universe.current_page_nodes().is_empty());

    let local = universe.intern_glue(glue(8));
    assert_panics(HandleClass::Glue, || {
        universe.prepend_page_contributions(vec![glue_node(local), foreign_node])
    });
    assert!(universe.page_contributions().is_empty());
}

#[test]
fn page_ingress_rejects_post_rollback_reuse() {
    let mut universe = Universe::new();
    let snapshot = universe.snapshot();
    let stale = universe.intern_glue(glue(9));
    universe.rollback(&snapshot);
    let replacement = universe.intern_glue(glue(10));
    assert_eq!(stale.raw(), replacement.raw());
    assert_ne!(stale, replacement);

    assert_panics(HandleClass::Glue, || {
        universe.append_page_contribution(glue_node(stale))
    });
    universe.append_page_contribution(glue_node(replacement));
    assert_eq!(universe.page_contributions().len(), 1);
}

fn exercise_rollback_reallocate(class: HandleClass) {
    match class {
        HandleClass::Symbol => {
            let mut universe = Universe::new();
            let snapshot = universe.snapshot();
            let stale = universe.intern("stale");
            universe.rollback(&snapshot);
            let replacement = universe.intern("replacement");
            assert_eq!(stale.raw(), replacement.raw(), "{class:?}");
            assert_ne!(stale, replacement, "{class:?}");
            assert_panics(class, || _ = universe.resolve(stale));
        }
        HandleClass::TokenList => {
            let mut universe = Universe::new();
            let snapshot = universe.snapshot();
            let stale = universe.intern_token_list(&[letter('s')]);
            universe.rollback(&snapshot);
            let replacement = universe.intern_token_list(&[letter('r')]);
            assert_eq!(stale.raw(), replacement.raw(), "{class:?}");
            assert_ne!(stale, replacement, "{class:?}");
            assert_panics(class, || _ = universe.tokens(stale));
        }
        HandleClass::MacroDefinition => {
            let mut universe = Universe::new();
            let snapshot = universe.snapshot();
            let stale = universe.intern_macro(empty_macro(MeaningFlags::LONG));
            universe.rollback(&snapshot);
            let replacement = universe.intern_macro(empty_macro(MeaningFlags::OUTER));
            assert_eq!(stale.raw(), replacement.raw(), "{class:?}");
            assert_ne!(stale, replacement, "{class:?}");
            assert_panics(class, || _ = universe.macro_definition(stale));
        }
        HandleClass::Glue => {
            let mut universe = Universe::new();
            let snapshot = universe.snapshot();
            let stale = universe.intern_glue(glue(1));
            universe.rollback(&snapshot);
            let replacement = universe.intern_glue(glue(2));
            assert_eq!(stale.raw(), replacement.raw(), "{class:?}");
            assert_ne!(stale, replacement, "{class:?}");
            assert_panics(class, || _ = universe.glue(stale));
        }
        HandleClass::Font => {
            let mut universe = Universe::new();
            let snapshot = universe.snapshot();
            let stale = universe.intern_font(font("stale", b"stale"));
            universe.rollback(&snapshot);
            let replacement = universe.intern_font(font("replacement", b"replacement"));
            assert_eq!(stale.raw(), replacement.raw(), "{class:?}");
            assert_ne!(stale, replacement, "{class:?}");
            assert_panics(class, || _ = universe.font(stale));
        }
        HandleClass::OriginList => {
            let mut universe = Universe::new();
            let snapshot = universe.snapshot();
            let stale = universe.allocate_origin_list(&[crate::token::OriginId::UNKNOWN]);
            universe.rollback(&snapshot);
            let replacement = universe.allocate_origin_list(&[crate::token::OriginId::UNKNOWN]);
            assert_eq!(stale.raw(), replacement.raw(), "{class:?}");
            assert_ne!(stale, replacement, "{class:?}");
            assert!(universe.origin_list_if_live(stale).is_none(), "{class:?}");
        }
        HandleClass::ArenaOrigin => {
            let mut universe = Universe::new();
            let snapshot = universe.snapshot();
            let stale = universe.synthetic_origin(SyntheticOriginKind::Primitive);
            universe.rollback(&snapshot);
            let replacement = universe.synthetic_origin(SyntheticOriginKind::Engine);
            assert_ne!(stale.raw(), replacement.raw(), "{class:?}");
            assert!(universe.origin_if_live(stale).is_none(), "{class:?}");
            assert!(universe.origin_if_live(replacement).is_some(), "{class:?}");
        }
        HandleClass::SourcePosition => {
            let mut universe = Universe::new();
            let snapshot = universe.snapshot();
            let stale = source_position(&mut universe, 0, b"s");
            universe.rollback(&snapshot);
            let replacement = source_position(&mut universe, 0, b"r");
            assert_ne!(stale, replacement, "{class:?}");
            assert!(universe.source_span(stale, stale).is_err(), "{class:?}");
            assert!(
                universe.source_span(replacement, replacement).is_ok(),
                "{class:?}"
            );
        }
        HandleClass::EpochNodeList => {
            let mut universe = Universe::new();
            let snapshot = universe.snapshot();
            let stale = universe.freeze_node_list(&[Node::Penalty(1)]);
            universe.rollback(&snapshot);
            let replacement = universe.freeze_node_list(&[Node::Penalty(2)]);
            assert_ne!(stale, replacement, "{class:?}");
            assert_panics(class, || _ = universe.nodes(stale));
        }
        HandleClass::SurvivorNodeList => {
            let mut universe = Universe::new();
            let snapshot = universe.snapshot();
            let stale = store_box(&mut universe, 0, 1);
            universe.rollback(&snapshot);
            let replacement = store_box(&mut universe, 0, 2);
            assert_ne!(stale.arena(), replacement.arena(), "{class:?}");
            assert_panics(class, || _ = universe.nodes(stale));
        }
        HandleClass::WorldInputRecord => {
            let mut universe = universe_with_files();
            let snapshot = universe.snapshot();
            let stale = universe
                .world_mut()
                .read_file("stale.tex")
                .expect("read stale input")
                .record();
            universe.rollback(&snapshot);
            let replacement = universe
                .world_mut()
                .read_file("replacement.tex")
                .expect("read replacement input")
                .record();
            assert_eq!(stale.raw(), replacement.raw(), "{class:?}");
            assert_ne!(stale, replacement, "{class:?}");
            assert!(universe.world().input_record(stale).is_none(), "{class:?}");
        }
    }
}

fn exercise_fork(class: HandleClass) {
    match class {
        HandleClass::Symbol => {
            let mut parent = Universe::new();
            let inherited = parent.intern("inherited");
            let mut child = parent.clone();
            assert_eq!(
                parent.resolve(inherited),
                child.resolve(inherited),
                "{class:?}"
            );
            let parent_only = parent.intern("parent");
            let child_only = child.intern("child");
            assert_panics(class, || _ = parent.resolve(child_only));
            assert_panics(class, || _ = child.resolve(parent_only));
        }
        HandleClass::TokenList => {
            let mut parent = Universe::new();
            let inherited = parent.intern_token_list(&[letter('i')]);
            let mut child = parent.clone();
            assert_eq!(
                parent.tokens(inherited),
                child.tokens(inherited),
                "{class:?}"
            );
            let parent_only = parent.intern_token_list(&[letter('p')]);
            let child_only = child.intern_token_list(&[letter('c')]);
            assert_panics(class, || _ = parent.tokens(child_only));
            assert_panics(class, || _ = child.tokens(parent_only));
        }
        HandleClass::MacroDefinition => {
            let mut parent = Universe::new();
            let inherited = parent.intern_macro(empty_macro(MeaningFlags::LONG));
            let mut child = parent.clone();
            assert_eq!(
                parent.macro_definition(inherited),
                child.macro_definition(inherited)
            );
            let parent_only = parent.intern_macro(empty_macro(MeaningFlags::OUTER));
            let child_only = child.intern_macro(empty_macro(MeaningFlags::PROTECTED));
            assert_panics(class, || _ = parent.macro_definition(child_only));
            assert_panics(class, || _ = child.macro_definition(parent_only));
        }
        HandleClass::Glue => {
            let mut parent = Universe::new();
            let inherited = parent.intern_glue(glue(1));
            let mut child = parent.clone();
            assert_eq!(parent.glue(inherited), child.glue(inherited), "{class:?}");
            let parent_only = parent.intern_glue(glue(2));
            let child_only = child.intern_glue(glue(3));
            assert_panics(class, || _ = parent.glue(child_only));
            assert_panics(class, || _ = child.glue(parent_only));
        }
        HandleClass::Font => {
            let mut parent = Universe::new();
            let inherited = parent.intern_font(font("inherited", b"inherited"));
            let mut child = parent.clone();
            assert_eq!(parent.font(inherited), child.font(inherited), "{class:?}");
            let parent_only = parent.intern_font(font("parent", b"parent"));
            let child_only = child.intern_font(font("child", b"child"));
            assert_panics(class, || _ = parent.font(child_only));
            assert_panics(class, || _ = child.font(parent_only));
        }
        HandleClass::OriginList => {
            let mut parent = Universe::new();
            let inherited = parent.allocate_origin_list(&[crate::token::OriginId::UNKNOWN]);
            let mut child = parent.clone();
            assert!(parent.origin_list_if_live(inherited).is_some(), "{class:?}");
            assert!(child.origin_list_if_live(inherited).is_some(), "{class:?}");
            let parent_only = parent.allocate_origin_list(&[crate::token::OriginId::UNKNOWN]);
            let child_only = child.allocate_origin_list(&[crate::token::OriginId::UNKNOWN]);
            assert!(
                parent.origin_list_if_live(child_only).is_none(),
                "{class:?}"
            );
            assert!(
                child.origin_list_if_live(parent_only).is_none(),
                "{class:?}"
            );
        }
        HandleClass::ArenaOrigin => {
            let mut parent = Universe::new();
            let inherited = parent.synthetic_origin(SyntheticOriginKind::Primitive);
            let mut child = parent.clone();
            assert!(parent.origin_if_live(inherited).is_some(), "{class:?}");
            assert!(child.origin_if_live(inherited).is_some(), "{class:?}");
            let parent_only = parent.synthetic_origin(SyntheticOriginKind::Format);
            let child_only = child.synthetic_origin(SyntheticOriginKind::Engine);
            assert!(parent.origin_if_live(child_only).is_none(), "{class:?}");
            assert!(child.origin_if_live(parent_only).is_none(), "{class:?}");
        }
        HandleClass::SourcePosition => {
            let mut parent = Universe::new();
            let inherited = source_position(&mut parent, 0, b"i");
            let mut child = parent.clone();
            assert!(
                parent.source_span(inherited, inherited).is_ok(),
                "{class:?}"
            );
            assert!(child.source_span(inherited, inherited).is_ok(), "{class:?}");
            let parent_only = source_position(&mut parent, 1, b"p");
            let child_only = source_position(&mut child, 1, b"c");
            assert!(
                parent.source_span(child_only, child_only).is_err(),
                "{class:?}"
            );
            assert!(
                child.source_span(parent_only, parent_only).is_err(),
                "{class:?}"
            );
        }
        HandleClass::EpochNodeList => {
            let mut parent = Universe::new();
            let inherited = parent.freeze_node_list(&[Node::Penalty(0)]);
            let mut child = parent.clone();
            assert_eq!(
                parent.nodes(inherited).to_vec(),
                child.nodes(inherited).to_vec()
            );
            let parent_only = parent.freeze_node_list(&[Node::Penalty(1)]);
            let child_only = child.freeze_node_list(&[Node::Penalty(2)]);
            assert_panics(class, || _ = parent.nodes(child_only));
            assert_panics(class, || _ = child.nodes(parent_only));
        }
        HandleClass::SurvivorNodeList => {
            let mut parent = Universe::new();
            let inherited = store_box(&mut parent, 0, 0);
            let mut child = parent.clone();
            assert_eq!(
                parent.nodes(inherited).to_vec(),
                child.nodes(inherited).to_vec()
            );
            let parent_only = store_box(&mut parent, 1, 1);
            let child_only = store_box(&mut child, 1, 2);
            assert_panics(class, || _ = parent.nodes(child_only));
            assert_panics(class, || _ = child.nodes(parent_only));
        }
        HandleClass::WorldInputRecord => {
            let mut parent = universe_with_files();
            let inherited = parent
                .world_mut()
                .read_file("inherited.tex")
                .expect("read inherited input")
                .record();
            let mut child = parent.clone();
            assert!(
                parent.world().input_record(inherited).is_some(),
                "{class:?}"
            );
            assert!(child.world().input_record(inherited).is_some(), "{class:?}");
            let parent_only = parent
                .world_mut()
                .read_file("parent.tex")
                .expect("read parent input")
                .record();
            let child_only = child
                .world_mut()
                .read_file("child.tex")
                .expect("read child input")
                .record();
            assert!(
                parent.world().input_record(child_only).is_none(),
                "{class:?}"
            );
            assert!(
                child.world().input_record(parent_only).is_none(),
                "{class:?}"
            );
        }
    }
}

fn exercise_cross_universe(class: HandleClass) {
    match class {
        HandleClass::Symbol => {
            let mut owner = Universe::new();
            let foreign = owner.intern("foreign");
            let other = Universe::new();
            assert_panics(class, || _ = other.resolve(foreign));
        }
        HandleClass::TokenList => {
            let mut owner = Universe::new();
            let foreign = owner.intern_token_list(&[letter('f')]);
            let other = Universe::new();
            assert_panics(class, || _ = other.tokens(foreign));
        }
        HandleClass::MacroDefinition => {
            let mut owner = Universe::new();
            let foreign = owner.intern_macro(empty_macro(MeaningFlags::LONG));
            let other = Universe::new();
            assert_panics(class, || _ = other.macro_definition(foreign));
        }
        HandleClass::Glue => {
            let mut owner = Universe::new();
            let foreign = owner.intern_glue(glue(1));
            let other = Universe::new();
            assert_panics(class, || _ = other.glue(foreign));
        }
        HandleClass::Font => {
            let mut owner = Universe::new();
            let foreign = owner.intern_font(font("foreign", b"foreign"));
            let other = Universe::new();
            assert_panics(class, || _ = other.font(foreign));
        }
        HandleClass::OriginList => {
            let mut owner = Universe::new();
            let foreign = owner.allocate_origin_list(&[crate::token::OriginId::UNKNOWN]);
            let other = Universe::new();
            assert!(other.origin_list_if_live(foreign).is_none(), "{class:?}");
        }
        HandleClass::ArenaOrigin => {
            let mut owner = Universe::new();
            let foreign = owner.synthetic_origin(SyntheticOriginKind::Primitive);
            let other = Universe::new();
            assert!(other.origin_if_live(foreign).is_none(), "{class:?}");
        }
        HandleClass::SourcePosition => {
            let mut owner = Universe::new();
            let foreign = source_position(&mut owner, 0, b"f");
            let other = Universe::new();
            assert!(other.source_span(foreign, foreign).is_err(), "{class:?}");
        }
        HandleClass::EpochNodeList => {
            let mut owner = Universe::new();
            let foreign = owner.freeze_node_list(&[Node::Penalty(1)]);
            let other = Universe::new();
            assert_panics(class, || _ = other.nodes(foreign));
        }
        HandleClass::SurvivorNodeList => {
            let mut owner = Universe::new();
            let foreign = store_box(&mut owner, 0, 1);
            let other = Universe::new();
            assert_panics(class, || _ = other.nodes(foreign));
        }
        HandleClass::WorldInputRecord => {
            let mut owner = universe_with_files();
            let foreign = owner
                .world_mut()
                .read_file("foreign.tex")
                .expect("read foreign input")
                .record();
            let other = World::memory();
            assert!(other.input_record(foreign).is_none(), "{class:?}");
        }
    }
}

fn assert_panics(class: HandleClass, f: impl FnOnce()) {
    assert!(catch_unwind(AssertUnwindSafe(f)).is_err(), "{class:?}");
}

fn letter(ch: char) -> Token {
    Token::Char {
        ch,
        cat: Catcode::Letter,
    }
}

fn empty_macro(flags: MeaningFlags) -> MacroMeaning {
    MacroMeaning::new(flags, TokenListId::EMPTY, TokenListId::EMPTY)
}

fn glue(width: i32) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(width),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    }
}

fn glue_node(spec: crate::ids::GlueId) -> Node {
    Node::Glue {
        spec,
        kind: GlueKind::Normal,
        leader: None,
    }
}

fn font(name: &str, bytes: &[u8]) -> LoadedFont {
    LoadedFont::new(
        name,
        format!("{name}.tfm"),
        ContentHash::from_bytes(bytes).bytes(),
        0,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        FontMetrics::default(),
    )
}

fn source_position(
    universe: &mut Universe,
    source: u32,
    bytes: &[u8],
) -> crate::source_map::SourcePos {
    universe
        .register_source(
            SourceId::new(source),
            SourceDescriptor::generated(Arc::from(bytes)),
        )
        .expect("register generated source");
    universe
        .source_position(SourceId::new(source), 0)
        .expect("registered source has a start position")
}

fn store_box(universe: &mut Universe, register: u16, penalty: i32) -> NodeListId {
    let epoch = universe.freeze_node_list(&[Node::Penalty(penalty)]);
    universe.set_box_reg(register, epoch);
    universe.box_reg(register).expect("stored box is non-void")
}

fn universe_with_files() -> Universe {
    let mut world = World::memory();
    for path in [
        "stale.tex",
        "replacement.tex",
        "inherited.tex",
        "parent.tex",
        "child.tex",
        "foreign.tex",
    ] {
        world
            .set_memory_file(path, path.as_bytes().to_vec())
            .expect("seed memory input");
    }
    Universe::with_world(world)
}
