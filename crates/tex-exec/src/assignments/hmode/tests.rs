use super::*;
use crate::executor::NoopExecHooks;
use crate::mode::PendingHChar;
use tex_expand::{NoopRecorder, ReadRecorder};
use tex_lex::MemoryInput;
use tex_state::hyphenation::ExceptionSpec;
use tex_state::interner::Symbol;
use tex_state::node::Node;
use tex_state::provenance::SyntheticOriginKind;
use tex_state::token::TracedTokenWord;

#[test]
fn non_character_accent_lookahead_replays_the_original_traced_token() {
    let mut stores = Universe::new();
    crate::install_unexpandable_primitives(&mut stores);
    let origin = stores.synthetic_origin(SyntheticOriginKind::Test);
    let closing_group = TracedTokenWord::pack(
        Token::Char {
            ch: '}',
            cat: Catcode::EndGroup,
        },
        origin,
    );
    let mut input = InputStack::new(MemoryInput::new(""));
    push_traced_tokens(&mut input, &mut stores, [closing_group]);

    let base = scan_accent_base(
        &mut ModeNest::new(),
        &mut input,
        &mut stores,
        &mut NoopRecorder,
        &mut NoopExecHooks,
        TracedTokenWord::pack(
            Token::Char {
                ch: '^',
                cat: Catcode::Other,
            },
            OriginId::UNKNOWN,
        ),
    )
    .expect("accent lookahead should recover");

    assert_eq!(base, None);
    let summary = input.summary();
    let mut resumed = InputStack::<MemoryInput>::from_summary(&summary, |_, _, _| {
        Ok::<_, core::convert::Infallible>(MemoryInput::new(""))
    })
    .expect("pushed-back token should be checkpoint-resumable");
    let replayed = resumed
        .next_traced_token(&mut stores)
        .expect("read replayed token")
        .expect("closing group should be backed up");
    assert_eq!(replayed, closing_group);
}

#[derive(Default)]
struct CountingRecorder(usize);

impl ReadRecorder for CountingRecorder {
    fn record_meaning(&mut self, _symbol: Symbol, _meaning: Meaning) {
        self.0 += 1;
    }
}

#[test]
fn accent_lookahead_runs_assignments_and_accepts_char_num() {
    let mut stores = Universe::new();
    tex_expand::install_expandable_primitives(&mut stores);
    crate::install_unexpandable_primitives(&mut stores);
    let mut input = InputStack::new(MemoryInput::new("\\count0=7 \\char65"));
    let mut recorder = CountingRecorder::default();

    let base = scan_accent_base(
        &mut ModeNest::new(),
        &mut input,
        &mut stores,
        &mut recorder,
        &mut NoopExecHooks,
        TracedTokenWord::pack(
            Token::Char {
                ch: '^',
                cat: Catcode::Other,
            },
            OriginId::UNKNOWN,
        ),
    )
    .expect("accent base should scan");

    assert_eq!(base, Some(b'A'));
    assert_eq!(stores.count(0), 7);
    assert!(recorder.0 >= 2, "lookahead meanings should be recorded");
}

#[test]
fn sentence_space_factor_does_not_jump_after_an_uppercase_letter() {
    let mut stores = Universe::new();
    stores.set_sfcode('.', 3000);
    let mut nest = ModeNest::new();

    update_space_factor(&mut nest, &stores, 'A');
    assert_eq!(nest.current_list().space_factor(), 999);

    update_space_factor(&mut nest, &stores, '.');
    assert_eq!(nest.current_list().space_factor(), 1000);

    update_space_factor(&mut nest, &stores, 'a');
    update_space_factor(&mut nest, &stores, '.');
    assert_eq!(nest.current_list().space_factor(), 3000);
}

#[test]
fn accent_delta_rounds_half_scaled_points_like_tex82() {
    assert_eq!(
        accent_delta(
            Scaled::from_raw(10),
            Scaled::from_raw(1),
            Scaled::from_raw(0),
            Scaled::from_raw(0),
            Scaled::from_raw(0),
            Scaled::from_raw(0),
        ),
        Scaled::from_raw(5)
    );
}

#[test]
fn paragraph_leading_accent_is_replayed_after_entering_horizontal_mode() {
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory());
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\relax \\f \\accent19 E"));
    let mut executor = crate::Executor::new();

    executor
        .run(&mut input, &mut stores)
        .expect("paragraph-leading accent should execute");

    assert_eq!(executor.nest().current_mode(), crate::Mode::Horizontal);
    let nodes = executor.nest().current_list().nodes();
    assert!(
        matches!(
            nodes,
            [
                Node::HList(_),
                Node::Kern {
                    kind: KernKind::Accent,
                    ..
                },
                Node::HList(_),
                Node::Kern {
                    kind: KernKind::Accent,
                    ..
                },
                Node::Char { ch: 'E', .. },
                ..
            ]
        ),
        "unexpected paragraph-leading accent nodes: {nodes:?}"
    );
    let Node::HList(accent_box) = &nodes[2] else {
        unreachable!("matched shifted accent box")
    };
    assert!(matches!(
        stores.nodes(accent_box.children).testing_decoded(),
        [Node::Char { ch, .. }] if *ch == char::from(19)
    ));
}

#[test]
fn unrestricted_reconstitution_inserts_null_disc_after_font_hyphen() {
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory());
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\relax \\f"));
    crate::Executor::new()
        .run(&mut input, &mut stores)
        .expect("font selection should execute");
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'), false);
    let pending: Vec<_> = "in-line"
        .chars()
        .map(|ch| PendingHChar { font, ch })
        .collect();

    let unrestricted = reconstitute(&mut stores, &pending, false, true);
    let restricted = reconstitute(&mut stores, &pending, false, false);

    assert!(matches!(
        unrestricted.as_slice(),
        [
            Node::Char { ch: 'i', .. },
            Node::Char { ch: 'n', .. },
            Node::Char { ch: '-', .. },
            Node::Disc {
                kind: DiscKind::ExplicitHyphen,
                ..
            },
            Node::Char { ch: 'l', .. },
            Node::Char { ch: 'i', .. },
            Node::Char { ch: 'n', .. },
            Node::Char { ch: 'e', .. },
        ]
    ));
    assert!(
        !restricted
            .iter()
            .any(|node| matches!(node, Node::Disc { .. }))
    );
}

#[test]
fn hyphenation_inside_ff_ligature_preserves_the_unbroken_ligature() {
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory());
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\relax \\f"));
    crate::Executor::new()
        .run(&mut input, &mut stores)
        .expect("font selection should execute");
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "difference".to_owned(),
        positions: vec![3],
    });
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'), false);
    let nodes: Vec<_> = "difference"
        .chars()
        .map(|ch| Node::Char { font, ch })
        .collect();

    let hyphenated = super::super::hyphenation::hyphenated_hlist(&mut stores, &nodes);
    let disc = hyphenated
        .iter()
        .find_map(|node| match node {
            Node::Disc {
                pre, post, replace, ..
            } => Some((*pre, *post, *replace)),
            _ => None,
        })
        .expect("the exception should create a discretionary");

    assert!(matches!(
        stores.nodes(disc.2).testing_decoded(),
        [Node::Lig {
            ch: '\u{b}',
            orig: ('f', 'f'),
            ..
        }]
    ));
    assert!(
        matches!(
            stores.nodes(disc.0).testing_decoded(),
            [Node::Char { ch: 'f', .. }, Node::Char { ch: '-', .. }]
        ),
        "unexpected pre-break nodes: {:?}",
        stores.nodes(disc.0).testing_decoded()
    );
    assert!(matches!(
        stores.nodes(disc.1).testing_decoded(),
        [Node::Char { ch: 'f', .. }]
    ));
}

#[test]
fn hyphenation_keeps_scanning_across_font_kerns() {
    const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
    let mut stores = Universe::with_world(tex_state::World::memory());
    crate::install_unexpandable_primitives(&mut stores);
    stores
        .world_mut()
        .set_memory_file("cmr10.tfm", CMR10.to_vec())
        .expect("seed cmr10");
    let mut input = InputStack::new(MemoryInput::new("\\font\\f=cmr10 \\relax \\f"));
    crate::Executor::new()
        .run(&mut input, &mut stores)
        .expect("font selection should execute");
    stores.add_hyphenation_exception(ExceptionSpec {
        word: "availability".to_owned(),
        positions: vec![5, 9],
    });
    let font = stores.current_font();
    stores.set_font_hyphen_char(font, i32::from(b'-'), false);
    let pending: Vec<_> = "availability"
        .chars()
        .map(|ch| PendingHChar { font, ch })
        .collect();
    let nodes = reconstitute(&mut stores, &pending, false, false);
    assert!(
        nodes.iter().any(|node| matches!(
            node,
            Node::Kern {
                kind: KernKind::Font,
                ..
            }
        )),
        "the fixture must exercise an internal font kern: {nodes:?}"
    );

    let hyphenated = super::super::hyphenation::hyphenated_hlist(&mut stores, &nodes);
    assert_eq!(
        hyphenated
            .iter()
            .filter(|node| matches!(node, Node::Disc { .. }))
            .count(),
        2,
        "both exception points must survive font-kern reconstitution: {hyphenated:?}"
    );
}
