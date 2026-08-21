use std::sync::Arc;

use tex_command::{
    CommandDeliveryBoundary, CommandObservation, CommandObserver, FontResource, InputReason,
    InputTransition, ObservedToken, RecoveryKind, RegisteredSourceKind, SourceRegistration,
};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam};
use tex_state::glue::Order;
use tex_state::node::{GlueKind, KernKind, Node, Whatsit};
use tex_state::page::{PageDimension, PageMark};
use tex_state::provenance::InsertedOriginKind;
use tex_state::scaled::Scaled;
use tex_state::token::{Token, TokenWord};
use tex_state::{CommandContext, GenerationBrand, InputOpenState, ResolvedMeaning, Universe};

use super::{MainControl, MainControlStep};

#[derive(Clone, Debug, Eq, PartialEq)]
enum Shape {
    Char(char),
    Lig(Vec<char>),
    Kern(i32, KernKind),
    Glue {
        width: i32,
        stretch: i32,
        stretch_order: Order,
        shrink: i32,
        shrink_order: Order,
        kind: GlueKind,
        leader: bool,
    },
    Penalty(i32),
    Rule(Option<i32>, Option<i32>, Option<i32>),
    HBox {
        width: i32,
        height: i32,
        depth: i32,
        shift: i32,
        children: Vec<Shape>,
    },
    VBox {
        width: i32,
        height: i32,
        depth: i32,
        shift: i32,
        children: Vec<Shape>,
    },
    Mark(u16, String),
    Insert {
        class: u16,
        size: i32,
        split_top_skip: i32,
        split_max_depth: i32,
        floating_penalty: i32,
        content: Vec<Shape>,
    },
    Adjust(Vec<Shape>),
    Language(u8, u8, u8),
    MathOn(i32),
    MathOff(i32),
    Other(&'static str),
}

fn with_run<R>(
    source: &[u8],
    with_font: bool,
    test: impl for<'id> FnOnce(
        &MainControl<GenerationBrand<'id>>,
        &mut Universe<GenerationBrand<'id>>,
    ) -> R,
) -> R {
    crate::test_harness::with_plain_universe(|universe| {
        let mut control = MainControl::tex82_initex(universe);
        control.set_fuel_limit(100_000).expect("bounded fuel");
        if with_font {
            const CMR10: &[u8] =
                include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
            const CMSY10: &[u8] =
                include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");
            const CMEX10: &[u8] =
                include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmex10.tfm");
            universe
                .world_mut()
                .set_memory_file("cmr10.tfm", CMR10.to_vec())
                .expect("font fixture installs");
            universe
                .world_mut()
                .set_memory_file("cmr10b.tfm", CMR10.to_vec())
                .expect("second font fixture installs");
            universe
                .world_mut()
                .set_memory_file("cmsy10.tfm", CMSY10.to_vec())
                .expect("math symbol font fixture installs");
            universe
                .world_mut()
                .set_memory_file("cmex10.tfm", CMEX10.to_vec())
                .expect("math extension font fixture installs");
            let metrics = tex_state::InputReadState::read_input_file(
                &mut universe.input_open_context(),
                std::path::Path::new("cmr10.tfm"),
            )
            .expect("font fixture reads");
            control.capabilities_mut().register_font(
                "cmr10.tfm",
                FontResource::Tfm {
                    metrics,
                    opentype: None,
                },
            );
            let second_metrics = tex_state::InputReadState::read_input_file(
                &mut universe.input_open_context(),
                std::path::Path::new("cmr10b.tfm"),
            )
            .expect("second font fixture reads");
            control.capabilities_mut().register_font(
                "cmr10b.tfm",
                FontResource::Tfm {
                    metrics: second_metrics,
                    opentype: None,
                },
            );
            let symbol_metrics = tex_state::InputReadState::read_input_file(
                &mut universe.input_open_context(),
                std::path::Path::new("cmsy10.tfm"),
            )
            .expect("math symbol font fixture reads");
            control.capabilities_mut().register_font(
                "cmsy10.tfm",
                FontResource::Tfm {
                    metrics: symbol_metrics,
                    opentype: None,
                },
            );
            let extension_metrics = tex_state::InputReadState::read_input_file(
                &mut universe.input_open_context(),
                std::path::Path::new("cmex10.tfm"),
            )
            .expect("math extension font fixture reads");
            control.capabilities_mut().register_font(
                "cmex10.tfm",
                FontResource::Tfm {
                    metrics: extension_metrics,
                    opentype: None,
                },
            );
        }
        let registered = control
            .command_mut()
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(source),
            ))
            .expect("source registers");
        control
            .command_mut()
            .open_registered_source(registered)
            .expect("source opens");
        while let MainControlStep::Continue = control.step(universe).expect("program executes") {}
        test(&control, universe)
    })
}

#[derive(Default)]
struct Observations(Vec<CommandObservation>);

impl CommandObserver for Observations {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn with_run_observed<R>(
    source: &[u8],
    with_font: bool,
    test: impl for<'id> FnOnce(
        &MainControl<GenerationBrand<'id>>,
        &mut Universe<GenerationBrand<'id>>,
        Observations,
    ) -> R,
) -> R {
    crate::test_harness::with_plain_universe(|universe| {
        let mut control = MainControl::tex82_initex(universe);
        control.set_fuel_limit(100_000).expect("bounded fuel");
        if with_font {
            const CMR10: &[u8] =
                include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
            universe
                .world_mut()
                .set_memory_file("cmr10.tfm", CMR10.to_vec())
                .expect("font fixture installs");
            let metrics = tex_state::InputReadState::read_input_file(
                &mut universe.input_open_context(),
                std::path::Path::new("cmr10.tfm"),
            )
            .expect("font fixture reads");
            control.capabilities_mut().register_font(
                "cmr10.tfm",
                FontResource::Tfm {
                    metrics,
                    opentype: None,
                },
            );
        }
        let registered = control
            .command_mut()
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(source),
            ))
            .expect("source registers");
        control
            .command_mut()
            .open_registered_source(registered)
            .expect("source opens");
        let mut observations = Observations::default();
        while let MainControlStep::Continue = control
            .step_with_observer(universe, &mut observations)
            .expect("program executes")
        {}
        test(&control, universe, observations)
    })
}

fn with_run_until_count<R>(
    source: &[u8],
    expected_count: i32,
    test: impl for<'id> FnOnce(
        &MainControl<GenerationBrand<'id>>,
        &mut Universe<GenerationBrand<'id>>,
    ) -> R,
) -> R {
    crate::test_harness::with_plain_universe(|universe| {
        let mut control = MainControl::tex82_initex(universe);
        control.set_fuel_limit(100_000).expect("bounded fuel");
        let registered = control
            .command_mut()
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(source),
            ))
            .expect("source registers");
        control
            .command_mut()
            .open_registered_source(registered)
            .expect("source opens");
        while count_register(universe, 0) != expected_count {
            assert_eq!(
                control.step(universe).expect("program executes"),
                MainControlStep::Continue,
                "source ended before its live-mode probe"
            );
        }
        test(&control, universe)
    })
}

fn word_text(words: &[TokenWord]) -> String {
    words
        .iter()
        .filter_map(|word| match word.token() {
            Some(Token::Char { ch, .. }) => Some(ch),
            _ => None,
        })
        .collect()
}

fn rooted_text(tokens: &tex_state::node::NodeTokenList) -> String {
    word_text(tokens.words())
}

fn macro_text<G>(universe: &mut Universe<G>, name: &str) -> String {
    crate::test_harness::with_admitted(universe, |context| {
        let symbol = context.symbol(name).expect("probe macro is defined");
        let ResolvedMeaning::Macro { definition, .. } = context.meaning(symbol) else {
            panic!("probe is a macro")
        };
        word_text(context.definition(definition).replacement_text())
    })
}

fn page_mark_text<G>(universe: &mut Universe<G>, mark: PageMark) -> String {
    crate::test_harness::with_admitted(universe, |context| {
        word_text(context.token_list(context.page_mark(mark)))
    })
}

fn page_mark_is_none<G>(universe: &mut Universe<G>, mark: PageMark) -> bool {
    crate::test_harness::with_admitted(universe, |context| context.page_mark_value(mark).is_none())
}

fn page_vec_context<G>(
    context: &CommandContext<'_, G>,
    root: tex_state::node_arena::PageListId,
) -> Vec<Node> {
    context
        .page_node_list(root)
        .expect("test list belongs to the page arena")
        .nodes()
        .to_vec()
}

fn shapes_context<G>(context: &CommandContext<'_, G>, nodes: &[Node]) -> Vec<Shape> {
    nodes
        .iter()
        .map(|node| match node {
            Node::Char { ch, .. } => Shape::Char(*ch),
            Node::Lig { orig, .. } => Shape::Lig(orig.clone()),
            Node::Kern { amount, kind } => Shape::Kern(amount.raw(), *kind),
            Node::Glue {
                spec, kind, leader, ..
            } => Shape::Glue {
                width: spec.width.raw(),
                stretch: spec.stretch.raw(),
                stretch_order: spec.stretch_order,
                shrink: spec.shrink.raw(),
                shrink_order: spec.shrink_order,
                kind: *kind,
                leader: leader.is_some(),
            },
            Node::Penalty(value) => Shape::Penalty(*value),
            Node::Rule {
                width,
                height,
                depth,
            } => Shape::Rule(
                width.map(Scaled::raw),
                height.map(Scaled::raw),
                depth.map(Scaled::raw),
            ),
            Node::HList(boxed) => Shape::HBox {
                width: boxed.width.raw(),
                height: boxed.height.raw(),
                depth: boxed.depth.raw(),
                shift: boxed.shift.raw(),
                children: shapes_context(context, &page_vec_context(context, boxed.children)),
            },
            Node::VList(boxed) => Shape::VBox {
                width: boxed.width.raw(),
                height: boxed.height.raw(),
                depth: boxed.depth.raw(),
                shift: boxed.shift.raw(),
                children: shapes_context(context, &page_vec_context(context, boxed.children)),
            },
            Node::Mark { class, tokens } => Shape::Mark(*class, rooted_text(tokens)),
            Node::Ins {
                class,
                size,
                split_top_skip,
                split_max_depth,
                floating_penalty,
                content,
            } => Shape::Insert {
                class: *class,
                size: size.raw(),
                split_top_skip: split_top_skip.width.raw(),
                split_max_depth: split_max_depth.raw(),
                floating_penalty: *floating_penalty,
                content: shapes_context(context, &page_vec_context(context, *content)),
            },
            Node::Adjust(adjust) => Shape::Adjust(shapes_context(
                context,
                &page_vec_context(context, adjust.content),
            )),
            Node::Whatsit(Whatsit::Language {
                language,
                left_hyphen_min,
                right_hyphen_min,
            }) => Shape::Language(*language, *left_hyphen_min, *right_hyphen_min),
            Node::MathOn(amount) => Shape::MathOn(amount.raw()),
            Node::MathOff(amount) => Shape::MathOff(amount.raw()),
            Node::Disc { .. } => Shape::Other("disc"),
            Node::Whatsit(_) => Shape::Other("whatsit"),
            Node::Direction(_) => Shape::Other("direction"),
            Node::MathNoad(_)
            | Node::FractionNoad(_)
            | Node::MathStyle(_)
            | Node::MathChoice(_)
            | Node::MathList(_)
            | Node::Nonscript => Shape::Other("math"),
            Node::MarginKern { .. } => Shape::Other("margin-kern"),
            Node::Unset(_) => Shape::Other("unset"),
        })
        .collect()
}

fn register_shapes<G>(universe: &mut Universe<G>, register: u16) -> Option<Vec<Shape>> {
    crate::test_harness::with_admitted(universe, |context| {
        let root = context.copy_box_to_page(register)?;
        Some(shapes_context(context, &page_vec_context(context, root)))
    })
}

fn page_vec<G>(universe: &mut Universe<G>, root: tex_state::node_arena::PageListId) -> Vec<Node> {
    crate::test_harness::with_admitted(universe, |context| page_vec_context(context, root))
}

fn shapes<G>(universe: &mut Universe<G>, nodes: &[Node]) -> Vec<Shape> {
    crate::test_harness::with_admitted(universe, |context| shapes_context(context, nodes))
}

fn boxed_children<G>(universe: &mut Universe<G>, register: u16) -> Vec<Node> {
    crate::test_harness::with_admitted(universe, |context| {
        let root = context
            .copy_box_to_page(register)
            .expect("box register is nonvoid");
        let nodes = page_vec_context(context, root);
        let [Node::HList(boxed) | Node::VList(boxed)] = nodes.as_slice() else {
            panic!("box register has exactly one box root")
        };
        page_vec_context(context, boxed.children)
    })
}

fn terminal<G>(universe: &Universe<G>) -> String {
    super::terminal_text(universe)
}

fn count_register<G>(universe: &mut Universe<G>, register: u16) -> i32 {
    crate::test_harness::with_admitted(universe, |context| {
        context
            .count(register)
            .expect("count register belongs to the admitted state")
    })
}

fn dimen_parameter<G>(universe: &mut Universe<G>, parameter: DimenParam) -> Scaled {
    crate::test_harness::with_admitted(universe, |context| context.dimen_param(parameter))
}

fn int_parameter<G>(universe: &mut Universe<G>, parameter: IntParam) -> i32 {
    crate::test_harness::with_admitted(universe, |context| context.int_param(parameter))
}

fn glue_parameter<G>(
    universe: &mut Universe<G>,
    parameter: GlueParam,
) -> tex_state::glue::GlueSpec {
    crate::test_harness::with_admitted(universe, |context| {
        let glue = context
            .glue_param(parameter)
            .expect("glue parameter is defined");
        context.glue(glue)
    })
}

fn dimen_register<G>(universe: &mut Universe<G>, register: u16) -> Scaled {
    crate::test_harness::with_admitted(universe, |context| context.dimen(register))
}

#[derive(Debug)]
struct DetachedPageProbe {
    contents: tex_state::page::PageContents,
    contributions: Vec<Node>,
    depth: Scaled,
}

fn detach_page_probe<G>(universe: &mut Universe<G>) -> DetachedPageProbe {
    crate::test_harness::with_admitted(universe, |context| DetachedPageProbe {
        contents: context.page_contents(),
        contributions: context.page_contributions().iter().cloned().collect(),
        depth: context.page_dimension(PageDimension::Depth),
    })
}

fn page_last_penalty<G>(universe: &mut Universe<G>) -> i32 {
    crate::test_harness::with_admitted(universe, |context| page_last_penalty(&mut context))
}

fn outer_vertical_shapes<G>(universe: &mut Universe<G>) -> Vec<Shape> {
    crate::test_harness::with_admitted(universe, |context| {
        let nodes = context
            .current_page_nodes()
            .into_iter()
            .chain(context.page_contributions().iter().cloned())
            .collect::<Vec<_>>();
        shapes_context(context, &nodes)
    })
}

#[test]
fn vsplit_void_nonvbox_pruning_marks_and_packaging_matrix() {
    // TeX82 §§977--979: exercise both no-op exits, prefix ownership, all
    // split-mark transitions, top pruning, exact/oversized packing, and the
    // source-register replacement contract through ordered node projections.
    with_run(br"\setbox1=\vsplit0 to5pt", false, |_, mut void| {
        assert_eq!(register_shapes(&mut void, 0), None);
        assert_eq!(register_shapes(&mut void, 1), None);
    });

    with_run(
        br"\nonstopmode\setbox0=\hbox{\kern7pt}\setbox1=\vsplit0 to5pt",
        false,
        |_, mut wrong| {
            assert!(matches!(
                register_shapes(&mut wrong, 0).as_deref(),
                Some([Shape::HBox { children, .. }])
                    if children.as_slice() == [Shape::Kern(7 * Scaled::UNITY, KernKind::Explicit)]
            ));
            assert_eq!(register_shapes(&mut wrong, 1), None);
            assert!(terminal(&wrong).contains("vbox"), "{}", terminal(&wrong));
        },
    );

    with_run(
        br"\splittopskip=1pt
          \setbox0=\vbox{\mark{a}\hrule height4pt\mark{b}\penalty-10000
                           \kern2pt\mark{c}\hrule height6pt}
          \setbox1=\vsplit0 to4pt",
        false,
        |_, mut split| {
            let prefix = register_shapes(&mut split, 1).expect("split prefix");
            let remainder = register_shapes(&mut split, 0).expect("split remainder");
            assert!(
                matches!(prefix.as_slice(), [Shape::VBox { children, .. }]
        if children.as_slice() == [Shape::Mark(0, "a".into()), Shape::Rule(None, Some(4 * Scaled::UNITY), Some(0)), Shape::Mark(0, "b".into())]),
                "prefix={prefix:?}; remainder={remainder:?}"
            );
            assert!(
                matches!(remainder.as_slice(), [Shape::VBox { children, .. }]
        if matches!(children.as_slice(), [Shape::Mark(0, mark), Shape::Glue { kind: GlueKind::SplitTopSkip, leader: false, .. }, Shape::Rule(None, Some(height), Some(0))] if mark == "c" && *height == 6 * Scaled::UNITY))
            );
            for mark in [PageMark::SplitFirst, PageMark::SplitBot] {
                assert_eq!(
                    page_mark_text(&mut split, mark),
                    if mark == PageMark::SplitFirst {
                        "a"
                    } else {
                        "b"
                    }
                );
            }
        },
    );

    with_run(
        br"\setbox0=\vbox{\hrule height3pt}\setbox1=\vsplit0 to30pt",
        false,
        |_, mut complete| {
            assert_eq!(register_shapes(&mut complete, 0), None);
            assert!(
                matches!(register_shapes(&mut complete, 1).as_deref(), Some([Shape::VBox { height, children, .. }]) if *height == 30 * Scaled::UNITY && matches!(children.as_slice(), [Shape::Rule(_, Some(h), _) ] if *h == 3 * Scaled::UNITY))
            );
        },
    );
}

#[test]
fn vsplit_breakpoint_mark_scope_and_complete_ownership_matrix() {
    // TeX82 §§977--979: split marks are reset before every attempt, the
    // selected breakpoint is removed rather than shared, nested boxes remain
    // atomic, and ordinary save-stack restoration owns both source and target
    // registers independently of the newly packed split graph.
    with_run(
        br"\setbox0=\vbox{\mark{old}\hrule height1pt\penalty-10000}
          \setbox1=\vsplit0 to1pt
          \setbox0=\vbox{\hrule height1pt}\setbox2=\vsplit0 to2pt",
        false,
        |_, cleared| {
            assert!(page_mark_is_none(&mut cleared, PageMark::SplitFirst));
            assert!(page_mark_is_none(&mut cleared, PageMark::SplitBot));

            with_run(
                br"\setbox0=\vbox{\penalty-10000\mark{tail}\hrule height2pt}
          \setbox1=\vsplit0 to0pt",
                false,
                |_, mut first| {
                    assert_eq!(
                        register_shapes(&mut first, 1),
                        Some(vec![Shape::VBox {
                            width: 0,
                            height: 0,
                            depth: 0,
                            shift: 0,
                            children: vec![],
                        }])
                    );
                    assert!(matches!(
                        register_shapes(&mut first, 0).as_deref(),
                        Some([Shape::VBox { children, .. }])
                            if matches!(children.as_slice(), [Shape::Mark(0, mark), Shape::Glue { kind: GlueKind::SplitTopSkip, .. }, Shape::Rule(_, Some(height), _) ] if mark == "tail" && *height == 2 * Scaled::UNITY)
                    ));

                    with_run(
                        br"\splittopskip=0pt
          \setbox0=\vbox{\mark{first}\hrule height2pt\mark{middle}
                           \penalty-10000\kern3pt\mark{tail}\hrule height4pt}
          \setbox1=\vsplit0 to2pt",
                        false,
                        |_, mut middle| {
                            assert!(matches!(
                                register_shapes(&mut middle, 1).as_deref(),
                                Some([Shape::VBox { children, .. }])
                                    if children.as_slice() == [
                                        Shape::Mark(0, "first".into()),
                                        Shape::Rule(None, Some(2 * Scaled::UNITY), Some(0)),
                                        Shape::Mark(0, "middle".into()),
                                    ]
                            ));
                            assert!(matches!(
                                register_shapes(&mut middle, 0).as_deref(),
                                Some([Shape::VBox { children, .. }])
                                    if children.as_slice() == [
                                        Shape::Mark(0, "tail".into()),
                                        Shape::Glue {
                                            width: 0,
                                            stretch: 0,
                                            stretch_order: Order::Normal,
                                            shrink: 0,
                                            shrink_order: Order::Normal,
                                            kind: GlueKind::SplitTopSkip,
                                            leader: false,
                                        },
                                        Shape::Rule(None, Some(4 * Scaled::UNITY), Some(0)),
                                    ]
                            ));
                            assert_eq!(page_mark_text(&mut middle, PageMark::SplitFirst), "first");
                            assert_eq!(page_mark_text(&mut middle, PageMark::SplitBot), "middle");

                            with_run(
                                br"\setbox0=\vbox{\hrule height1pt\penalty-10000\penalty-9999
                           \hrule height2pt}
          \setbox1=\vsplit0 to1pt",
                                false,
                                |_, mut penalty_first| {
                                    assert!(matches!(
                                        register_shapes(&mut penalty_first, 1).as_deref(),
                                        Some([Shape::VBox { children, .. }])
                                            if children.as_slice() == [Shape::Rule(None, Some(Scaled::UNITY), Some(0))]
                                    ));
                                    assert!(
                                        matches!(
                                            register_shapes(&mut penalty_first, 0).as_deref(),
                                            Some([Shape::VBox { children, .. }])
                                                if matches!(children.as_slice(), [Shape::Glue { kind: GlueKind::SplitTopSkip, .. }, Shape::Rule(_, Some(height), _)] if *height == 2 * Scaled::UNITY)
                                        ),
                                        "{:?}",
                                        register_shapes(&mut penalty_first, 0)
                                    );

                                    with_run(
        br"\setbox0=\vbox{\vbox{\mark{nested}\hrule height1pt\penalty-10000
                                  \hrule height1pt}
                           \penalty-10000\mark{outer-tail}\hrule height3pt}
          \setbox1=\vsplit0 to2pt",
        false, |_, mut nested| {
    assert!(matches!(
        register_shapes(&mut nested, 1).as_deref(),
        Some([Shape::VBox { children, .. }])
            if matches!(children.as_slice(), [Shape::VBox { children: inner, .. }]
                if inner.as_slice() == [
                    Shape::Mark(0, "nested".into()),
                    Shape::Rule(None, Some(Scaled::UNITY), Some(0)),
                    Shape::Penalty(-10_000),
                    Shape::Rule(None, Some(Scaled::UNITY), Some(0)),
                ])
    ));
    assert!(page_mark_is_none(&mut nested, PageMark::SplitFirst));
    assert!(page_mark_is_none(&mut nested, PageMark::SplitBot));
    assert!(matches!(
        register_shapes(&mut nested, 0).as_deref(),
        Some([Shape::VBox { children, .. }])
            if matches!(children.as_slice(), [Shape::Mark(0, mark), Shape::Glue { kind: GlueKind::SplitTopSkip, .. }, Shape::Rule(_, Some(height), _)] if mark == "outer-tail" && *height == 3 * Scaled::UNITY)
    ));

    with_run(
        br"\setbox0=\vbox{\hrule height8pt}\setbox1=\vbox{\kern9pt}
          {\setbox0=\vbox{\mark{local}\hrule height2pt\penalty-10000
                            \kern3pt\hrule height4pt}
           \global\setbox2=\vsplit0 to2pt
           \setbox1=\copy0}",
        false, |_, mut scoped| {
    assert!(matches!(
        register_shapes(&mut scoped, 0).as_deref(),
        Some([Shape::VBox { children, .. }])
            if children.as_slice() == [Shape::Rule(None, Some(8 * Scaled::UNITY), Some(0))]
    ));
    assert!(matches!(
        register_shapes(&mut scoped, 1).as_deref(),
        Some([Shape::VBox { children, .. }])
            if children.as_slice() == [Shape::Kern(9 * Scaled::UNITY, KernKind::Explicit)]
    ));
    assert!(matches!(
        register_shapes(&mut scoped, 2).as_deref(),
        Some([Shape::VBox { children, .. }])
            if children.as_slice() == [
                Shape::Mark(0, "local".into()),
                Shape::Rule(None, Some(2 * Scaled::UNITY), Some(0)),
            ]
    ));

    });
    });
                                },
                            );
                        },
                    );
                },
            );
        },
    );
}

#[test]
fn text_material_character_ligkern_space_language_and_vertical_replay_matrix() {
    // TeX82 §§1032--1044: ordered projections distinguish every character
    // delivery form, ligature/kern and no-boundary handling, all space-glue
    // sources, language nodes, missing glyph recovery, and vertical replay.
    with_run(
        br"\font\f=cmr10 \f \chardef\C=67
          \setbox0=\hbox{A\char66\C}
          \setbox1=\hbox{fi AV f\noboundary i}
          \spaceskip=4pt plus2pt minus1pt \xspaceskip=9pt
          \setbox2=\hbox{A\spacefactor=500\relax{} X}
          \setbox13=\hbox{A\spacefactor=3000\relax{} X}
          \lefthyphenmin=2 \righthyphenmin=3
          \setbox3=\hbox{\setlanguage7\setlanguage7}
          \sfcode65=0 \sfcode66=500 \sfcode67=1000 \sfcode68=3000
          \setbox6=\hbox{A\xdef\sfzero{\the\spacefactor}}
          \setbox7=\hbox{B\xdef\sflow{\the\spacefactor}}
          \setbox8=\hbox{C\xdef\sfnormal{\the\spacefactor}}
          \setbox9=\hbox{D\xdef\sfhigh{\the\spacefactor}}
          \font\g=cmr10b \setbox10=\hbox{f\g i}
          \everypar{\global\advance\count0 by1}
          \setbox4=\vbox{A\par}
          \tracinglostchars=1\nullfont\setbox5=\hbox{A\kern1pt}",
        true,
        |_, mut universe| {
            assert_eq!(
                register_shapes(&mut universe, 0),
                Some(vec![Shape::HBox {
                    width: register_box_width(&mut universe, 0),
                    height: register_box_height(&mut universe, 0),
                    depth: register_box_depth(&mut universe, 0),
                    shift: 0,
                    children: vec![Shape::Char('A'), Shape::Char('B'), Shape::Char('C')],
                }])
            );
            let ligkern = register_shapes(&mut universe, 1).expect("lig/kern box");
            assert!(
                matches!(ligkern.as_slice(), [Shape::HBox { children, .. }]
        if matches!(children.as_slice(), [Shape::Lig(first), Shape::Glue { leader: false, .. }, Shape::Char('A'), Shape::Kern(_, KernKind::Font), Shape::Char('V'), Shape::Glue { leader: false, .. }, Shape::Char('f'), Shape::Char('i')] if first == &['f', 'i'])),
                "{ligkern:?}"
            );
            assert_eq!(macro_text(&mut universe, "sfzero"), "1000");
            assert_eq!(macro_text(&mut universe, "sflow"), "500");
            assert_eq!(macro_text(&mut universe, "sfnormal"), "1000");
            assert_eq!(macro_text(&mut universe, "sfhigh"), "3000");
            assert!(
                matches!(register_shapes(&mut universe, 10).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::Char('f'), Shape::Char('i')])
            );
            assert!(
                matches!(register_shapes(&mut universe, 2).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Char('A'), Shape::Glue { width, leader: false, .. }, Shape::Char('X')] if *width == 4 * Scaled::UNITY)),
                "{:?}",
                register_shapes(&mut universe, 2)
            );
            assert!(
                matches!(register_shapes(&mut universe, 13).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Char('A'), Shape::Glue { width, leader: false, .. }, Shape::Char('X')] if *width == 9 * Scaled::UNITY)),
                "{:?}",
                register_shapes(&mut universe, 13)
            );
            assert!(
                matches!(register_shapes(&mut universe, 3).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::Language(7, 2, 3), Shape::Language(7, 2, 3)])
            );
            assert_eq!(count_register(&mut universe, 0), 1);
            assert!(
                matches!(register_shapes(&mut universe, 4).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }] if line.contains(&Shape::Char('A'))))
            );
            assert!(
                matches!(register_shapes(&mut universe, 5).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::Kern(Scaled::UNITY, KernKind::Explicit)])
            );
            assert!(terminal(&universe).contains("Missing character"));
        },
    );
}

#[test]
fn text_boundary_font_glue_scaling_and_cache_matrix() {
    // TeX82 §§1032--1042: boundary suppression changes the ligature program;
    // font glue and both explicit space parameters preserve all five typed
    // glue fields after space-factor scaling; equal results reuse one frozen
    // glue identity while distinct factors do not alias.
    with_run(
        br"\font\f=cmr10 \f
          \setbox0=\hbox{fi}\setbox1=\hbox{f\noboundary i}
          \setbox2=\hbox{A\spacefactor=500\relax{} X}
          \setbox3=\hbox{A\spacefactor=1000\relax{} X}
          \setbox4=\hbox{A\spacefactor=3000\relax{} X}
          \spaceskip=4pt plus2fil minus3fill
          \setbox5=\hbox{A\spacefactor=500\relax{} X}
          \setbox6=\hbox{A\spacefactor=1000\relax{} X}
          \xspaceskip=9pt plus6fill minus12fil
          \setbox7=\hbox{A\spacefactor=3000\relax{} X}
          \spaceskip=0pt \xspaceskip=0pt
          \setbox8=\hbox{A\spacefactor=1000\relax{} X X}
          \fontdimen2\f=6pt\setbox9=\hbox{A X}
          \font\g=cmr10b \g\setbox10=\hbox{A X}",
        true,
        |_, mut universe| {
            assert!(matches!(
                register_shapes(&mut universe, 0).as_deref(),
                Some([Shape::HBox { children, .. }])
                    if children.as_slice() == [Shape::Lig(vec!['f', 'i'])]
            ));
            assert!(matches!(
                register_shapes(&mut universe, 1).as_deref(),
                Some([Shape::HBox { children, .. }])
                    if children.as_slice() == [Shape::Char('f'), Shape::Char('i')]
            ));

            let expected_font = [
                (218_453, 54_613, 145_636),
                (218_453, 109_226, 72_818),
                (291_271, 327_678, 24_272),
            ];
            for (register, (width, stretch, shrink)) in (2_u16..=4).zip(expected_font) {
                assert!(
                    matches!(
                        register_shapes(&mut universe, register).as_deref(),
                        Some([Shape::HBox { children, .. }])
                            if matches!(children.as_slice(), [
                                Shape::Char('A'), Shape::Glue {
                                    width: actual_width,
                                    stretch: actual_stretch,
                                    stretch_order: Order::Normal,
                                    shrink: actual_shrink,
                                    shrink_order: Order::Normal,
                                    kind: GlueKind::Normal,
                                    leader: false,
                                }, Shape::Char('X')]
                                if *actual_width == width && *actual_stretch == stretch && *actual_shrink == shrink)
                    ),
                    "register {register}: {:?}",
                    register_shapes(&mut universe, register)
                );
            }
            for (register, width, stretch, stretch_order, shrink, shrink_order, kind) in [
                (5, 4, 1, Order::Fil, 6, Order::Fill, GlueKind::SpaceSkip),
                (6, 4, 2, Order::Fil, 3, Order::Fill, GlueKind::SpaceSkip),
                (7, 9, 6, Order::Fill, 12, Order::Fil, GlueKind::XSpaceSkip),
            ] {
                assert!(
                    matches!(
                        register_shapes(&mut universe, register).as_deref(),
                        Some([Shape::HBox { children, .. }])
                            if matches!(children.as_slice(), [
                                Shape::Char('A'), Shape::Glue {
                                    width: actual_width,
                                    stretch: actual_stretch,
                                    stretch_order: actual_stretch_order,
                                    shrink: actual_shrink,
                                    shrink_order: actual_shrink_order,
                                    kind: actual_kind,
                                    leader: false,
                                }, Shape::Char('X')]
                                if *actual_width == width * Scaled::UNITY
                                && *actual_stretch == stretch * Scaled::UNITY
                                && *actual_stretch_order == stretch_order
                                && *actual_shrink == shrink * Scaled::UNITY
                                && *actual_shrink_order == shrink_order
                                && *actual_kind == kind)
                    ),
                    "register {register}: {:?}",
                    register_shapes(&mut universe, register)
                );
            }
            let cached = boxed_children(&mut universe, 8);
            let [
                Node::Char { ch: 'A', .. },
                Node::Glue { spec: first, .. },
                Node::Char { ch: 'X', .. },
                Node::Glue { spec: second, .. },
                Node::Char { ch: 'X', .. },
            ] = cached.as_slice()
            else {
                panic!("cached font spaces have exact ordered ownership: {cached:?}")
            };
            assert_eq!(
                *first,
                tex_state::glue::GlueSpec {
                    width: Scaled::from_raw(218_453),
                    stretch: Scaled::from_raw(109_226),
                    stretch_order: Order::Normal,
                    shrink: Scaled::from_raw(72_818),
                    shrink_order: Order::Normal,
                }
            );
            assert_eq!(
                *second,
                tex_state::glue::GlueSpec {
                    width: Scaled::from_raw(218_453),
                    stretch: Scaled::from_raw(109_116),
                    stretch_order: Order::Normal,
                    shrink: Scaled::from_raw(72_890),
                    shrink_order: Order::Normal,
                },
                "uppercase X's sfcode 999 selects a distinct cached scaling variant"
            );
            let low_box = boxed_children(&mut universe, 2);
            let [
                Node::Char { .. },
                Node::Glue { spec: low, .. },
                Node::Char { .. },
            ] = low_box.as_slice()
            else {
                panic!("low-factor box has one glue")
            };
            let normal_box = boxed_children(&mut universe, 3);
            let [
                Node::Char { .. },
                Node::Glue { spec: normal, .. },
                Node::Char { .. },
            ] = normal_box.as_slice()
            else {
                panic!("normal-factor box has one glue")
            };
            assert_ne!(low, normal, "different scaled specs do not alias");
            assert!(matches!(
                register_shapes(&mut universe, 9).as_deref(),
                Some([Shape::HBox { children, .. }])
                    if matches!(children.as_slice(), [Shape::Char('A'), Shape::Glue { width, .. }, Shape::Char('X')] if *width == 6 * Scaled::UNITY)
            ));
            assert!(matches!(
                register_shapes(&mut universe, 10).as_deref(),
                Some([Shape::HBox { children, .. }])
                    if matches!(children.as_slice(), [Shape::Char('A'), Shape::Glue { width, .. }, Shape::Char('X')] if *width == 218_453)
            ));
        },
    );
}

#[test]
fn text_outer_vertical_math_illegal_meaning_and_trigger_provenance_matrix() {
    // TeX82 §§1032--1044: a character starts a paragraph in outer vertical
    // mode and becomes a math noad in math mode; `\noboundary` is illegal in
    // both modes. The horizontal case pins the exact macro expansion,
    // boundary cancellation, backed-up trigger, and resumed command order.
    with_run(
        br"\font\f=cmr10 \f\nonstopmode
          \everypar{\global\advance\count1 by1}
          #
          A\par
          \noboundary\par
          \setbox0=\hbox{$\noboundary$}
          $#$\par
          \xdef\noboundarymeaning{\meaning\noboundary}\count0=7",
        true,
        |control, mut modes| {
            assert_eq!(
                count_register(&mut modes, 1),
                3,
                "character, vertical no-boundary, and math shift each start a paragraph"
            );
            assert_eq!(control.current_mode(), crate::mode::Mode::Vertical);
            assert!(
                matches!(
                    register_shapes(&mut modes, 0).as_deref(),
                    Some([Shape::HBox { children, .. }])
                        if children.as_slice() == [Shape::MathOn(0), Shape::MathOff(0)]
                ),
                "{:?}",
                register_shapes(&mut modes, 0)
            );
            assert_eq!(count_register(&mut modes, 0), 7);
            assert_eq!(macro_text(&mut modes, "noboundarymeaning"), r"\noboundary");
            let terminal = terminal(&modes);
            assert_eq!(
                terminal
                    .matches("macro parameter character #' in vertical mode")
                    .count(),
                1,
                "{terminal}"
            );
            assert_eq!(
                terminal
                    .matches("macro parameter character #' in math mode")
                    .count(),
                1,
                "{terminal}"
            );
            assert_eq!(terminal.matches("noboundary' in").count(), 0, "{terminal}");

            let source = br"\def\emit{\noboundary}\everypar{\global\advance\count0 by1}\emit\par";
            with_run_observed(source, false, |control, mut universe, observations| {
                assert_eq!(count_register(&mut universe, 0), 1);
                assert_eq!(control.current_mode(), crate::mode::Mode::Vertical);
                #[derive(Debug, Eq, PartialEq)]
                enum TriggerEvent {
                    Delivery(
                        CommandDeliveryBoundary,
                        ObservedToken,
                        String,
                        u64,
                        Option<(u64, u64)>,
                        Option<u64>,
                    ),
                    Backup(InputTransition, InputReason),
                    Recovery(RecoveryKind, Vec<ObservedToken>),
                }
                let start = observations
                    .0
                    .iter()
                    .rposition(|event| {
                        matches!(event, CommandObservation::Command(command)
            if command.spelling == ObservedToken::ControlSequence("emit".into())
                && command.command == "call")
                    })
                    .expect("source invocation follows the definition");
                let projection: Vec<_> = observations.0[start..]
                    .iter()
                    .filter_map(|event| match event {
                        CommandObservation::Command(command)
                            if command.spelling
                                == ObservedToken::ControlSequence("noboundary".into())
                                || command.spelling
                                    == ObservedToken::ControlSequence("emit".into()) =>
                        {
                            Some(TriggerEvent::Delivery(
                                command.boundary,
                                command.spelling.clone(),
                                command.command.clone(),
                                command.provenance.delivery_sequence,
                                command
                                    .provenance
                                    .source_range
                                    .map(|range| (range.start(), range.end())),
                                command
                                    .provenance
                                    .source_location
                                    .map(|location| location.byte()),
                            ))
                        }
                        CommandObservation::Input(input)
                            if input.reason == InputReason::Backup
                                && (input.transition == InputTransition::Backup
                                    || input.transition == InputTransition::Retire) =>
                        {
                            Some(TriggerEvent::Backup(input.transition, input.reason))
                        }
                        CommandObservation::Recovery(recovery)
                            if recovery.kind == RecoveryKind::Backup =>
                        {
                            Some(TriggerEvent::Recovery(
                                recovery.kind,
                                recovery.tokens.clone(),
                            ))
                        }
                        _ => None,
                    })
                    .collect();
                let emit = ObservedToken::ControlSequence("emit".into());
                let no_boundary = ObservedToken::ControlSequence("noboundary".into());
                assert_eq!(
                    projection,
                    vec![
                        TriggerEvent::Delivery(
                            CommandDeliveryBoundary::Raw,
                            emit,
                            "call".into(),
                            0,
                            Some((59, 64)),
                            Some(63),
                        ),
                        TriggerEvent::Delivery(
                            CommandDeliveryBoundary::Raw,
                            no_boundary.clone(),
                            "no_boundary".into(),
                            1,
                            None,
                            None,
                        ),
                        TriggerEvent::Delivery(
                            CommandDeliveryBoundary::Expanded,
                            no_boundary.clone(),
                            "no_boundary".into(),
                            1,
                            None,
                            None,
                        ),
                        TriggerEvent::Backup(InputTransition::Backup, InputReason::Backup),
                        TriggerEvent::Recovery(RecoveryKind::Backup, vec![no_boundary.clone()]),
                        TriggerEvent::Delivery(
                            CommandDeliveryBoundary::Raw,
                            no_boundary.clone(),
                            "no_boundary".into(),
                            8,
                            None,
                            None,
                        ),
                        TriggerEvent::Delivery(
                            CommandDeliveryBoundary::Expanded,
                            no_boundary.clone(),
                            "no_boundary".into(),
                            8,
                            None,
                            None,
                        ),
                        TriggerEvent::Backup(InputTransition::Retire, InputReason::Backup),
                        TriggerEvent::Backup(InputTransition::Backup, InputReason::Backup),
                        TriggerEvent::Recovery(RecoveryKind::Backup, vec![no_boundary.clone()]),
                        TriggerEvent::Delivery(
                            CommandDeliveryBoundary::Raw,
                            no_boundary.clone(),
                            "no_boundary".into(),
                            0,
                            None,
                            None,
                        ),
                        TriggerEvent::Delivery(
                            CommandDeliveryBoundary::Expanded,
                            no_boundary,
                            "no_boundary".into(),
                            0,
                            None,
                            None,
                        ),
                        TriggerEvent::Backup(InputTransition::Retire, InputReason::Backup),
                    ]
                );
            });
        },
    );
}

fn register_box<G>(universe: &mut Universe<G>, register: u16) -> tex_state::node::BoxNode {
    crate::test_harness::with_admitted(universe, |context| {
        let root = context.copy_box_to_page(register).expect("box register");
        let nodes = page_vec_context(context, root);
        match nodes.as_slice() {
            [Node::HList(boxed) | Node::VList(boxed)] => boxed.clone(),
            other => panic!("register {register} root: {other:?}"),
        }
    })
}

fn register_box_width<G>(universe: &mut Universe<G>, register: u16) -> i32 {
    register_box(universe, register).width.raw()
}

fn register_box_height<G>(universe: &mut Universe<G>, register: u16) -> i32 {
    register_box(universe, register).height.raw()
}

fn register_box_depth<G>(universe: &mut Universe<G>, register: u16) -> i32 {
    register_box(universe, register).depth.raw()
}

#[test]
fn direct_material_modes_operands_page_boundary_and_group_clear_matrix() {
    // TeX82 §§1055--1062/1070: named/explicit forms, signed dimensions,
    // glue orders, rule keyword replacement, h/v/math routing, page building,
    // and normal-paragraph clearing are all independently observable.
    with_run(
        br"\setbox0=\hbox{\kern-1pt\hskip2pt plus3fil minus4fill
                           \vrule height1pt width2pt height5pt depth-1pt\hfil}
          \setbox1=\vbox{\kern-2pt\vskip3pt plus1fill\hrule width4pt height5pt}
          \parshape=1 1pt 9pt \hangindent=7pt \hangafter=3
          {\parshape=1 2pt 8pt \hangindent=6pt \hangafter=4}
          \par",
        false,
        |_, mut universe| {
            assert!(
                matches!(register_shapes(&mut universe, 0).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Glue { width, kind: GlueKind::Normal, leader: false, .. }, Shape::Rule(Some(rw), Some(rh), Some(rd)), Shape::Glue { kind: GlueKind::Normal, leader: false, .. }]
            if *k == -Scaled::UNITY && *width == 2 * Scaled::UNITY && *rw == 2 * Scaled::UNITY && *rh == 5 * Scaled::UNITY && *rd == -Scaled::UNITY))
            );
            let vertical = register_shapes(&mut universe, 1);
            assert!(
                matches!(vertical.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Glue { width, kind: GlueKind::Normal, leader: false, .. }, Shape::Rule(Some(rw), Some(rh), None)]
            if *k == -2 * Scaled::UNITY && *width == 3 * Scaled::UNITY && *rw == 4 * Scaled::UNITY && *rh == 5 * Scaled::UNITY))
                    || matches!(vertical.as_deref(), Some([Shape::VBox { children, .. }])
            if matches!(children.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Glue { width, kind: GlueKind::Normal, leader: false, .. }, Shape::Rule(Some(rw), Some(rh), Some(0))]
                if *k == -2 * Scaled::UNITY && *width == 3 * Scaled::UNITY && *rw == 4 * Scaled::UNITY && *rh == 5 * Scaled::UNITY)),
                "{vertical:?}"
            );
            assert!(universe.paragraph_shape().is_empty());
            assert_eq!(
                dimen_parameter(&mut universe, DimenParam::HANG_INDENT),
                Scaled::from_raw(0)
            );
            assert_eq!(int_parameter(&mut universe, IntParam::HANG_AFTER), 1);

            with_run(
                br"\vsize=1pt\topskip=0pt\hrule height2pt\penalty-10000\end",
                false,
                |_, page| {
                    assert_eq!(page.world().artifact_commits().len(), 1);

                    with_run(
                        br"\setbox0=\hbox{\vrule width1pt X\kern2pt}",
                        false,
                        |_, mut recovery| {
                            assert!(
                                matches!(register_shapes(&mut recovery, 0).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Rule(Some(w), _, _), Shape::Kern(k, KernKind::Explicit)] if *w == Scaled::UNITY && *k == 2 * Scaled::UNITY))
                            );
                        },
                    );
                },
            );
        },
    );
}

#[test]
fn direct_material_full_mode_named_glue_and_math_routing_matrix() {
    // TeX82 §§1055--1062: fixed glue names are exactly their explicit specs,
    // including independent stretch/shrink infinity orders. Horizontal,
    // internal-vertical, outer-vertical, and math dispatch each preserve the
    // command's typed node and perform only the mode transitions in §1090/§1095.
    with_run(
        br"\setbox0=\hbox{\hskip0pt plus1fil}\setbox1=\hbox{\hfil}
          \setbox2=\hbox{\hskip0pt plus1fill}\setbox3=\hbox{\hfill}
          \setbox4=\hbox{\hskip0pt plus1fil minus1fil}\setbox5=\hbox{\hss}
          \setbox6=\hbox{\hskip0pt plus-1fil}\setbox7=\hbox{\hfilneg}
          \setbox8=\vbox{\vskip0pt plus1fil}\setbox9=\vbox{\vfil}
          \setbox10=\vbox{\vskip0pt plus1fill}\setbox11=\vbox{\vfill}
          \setbox12=\vbox{\vskip0pt plus1fil minus1fil}\setbox13=\vbox{\vss}
          \setbox14=\vbox{\vskip0pt plus-1fil}\setbox15=\vbox{\vfilneg}",
        false,
        |_, mut named| {
            for (explicit, fixed) in [
                (0, 1),
                (2, 3),
                (4, 5),
                (6, 7),
                (8, 9),
                (10, 11),
                (12, 13),
                (14, 15),
            ] {
                assert_eq!(
                    register_shapes(&mut named, explicit),
                    register_shapes(&mut named, fixed),
                    "registers {explicit}/{fixed} are named/explicit equivalents"
                );
            }
            for (register, stretch, stretch_order, shrink, shrink_order) in [
                (1, 1, Order::Fil, 0, Order::Normal),
                (3, 1, Order::Fill, 0, Order::Normal),
                (5, 1, Order::Fil, 1, Order::Fil),
                (7, -1, Order::Fil, 0, Order::Normal),
                (9, 1, Order::Fil, 0, Order::Normal),
                (11, 1, Order::Fill, 0, Order::Normal),
                (13, 1, Order::Fil, 1, Order::Fil),
                (15, -1, Order::Fil, 0, Order::Normal),
            ] {
                assert!(
                    matches!(
                        register_shapes(&mut named, register).as_deref(),
                        Some([Shape::HBox { children, .. } | Shape::VBox { children, .. }])
                            if matches!(children.as_slice(), [Shape::Glue {
                                width: 0,
                                stretch: actual_stretch,
                                stretch_order: actual_stretch_order,
                                shrink: actual_shrink,
                                shrink_order: actual_shrink_order,
                                kind: GlueKind::Normal,
                                leader: false,
                            }] if *actual_stretch == stretch * Scaled::UNITY
                                && *actual_stretch_order == stretch_order
                                && *actual_shrink == shrink * Scaled::UNITY
                                && *actual_shrink_order == shrink_order)
                    ),
                    "register {register}: {:?}",
                    register_shapes(&mut named, register)
                );
            }

            with_run(
                br"\font\sy=cmsy10 \font\ex=cmex10
          \textfont2=\sy\scriptfont2=\sy\scriptscriptfont2=\sy
          \textfont3=\ex\scriptfont3=\ex\scriptscriptfont3=\ex
          \setbox0=\hbox{\kern1pt\hskip2pt plus3fil minus4fill
                           \vrule width5pt height6pt depth7pt\penalty8}
          \setbox1=\vbox{\kern-1pt\vskip2pt plus3fill minus4fil
                           \hrule width5pt height6pt depth7pt\penalty8}
          \setbox2=\hbox{$\kern1pt\hskip2pt plus3fil minus4fill
                           \vrule width5pt height6pt depth7pt\penalty8$}",
                true,
                |_, mut modes| {
                    let direct = [
                        Shape::Kern(Scaled::UNITY, KernKind::Explicit),
                        Shape::Glue {
                            width: 2 * Scaled::UNITY,
                            stretch: 3 * Scaled::UNITY,
                            stretch_order: Order::Fil,
                            shrink: 4 * Scaled::UNITY,
                            shrink_order: Order::Fill,
                            kind: GlueKind::Normal,
                            leader: false,
                        },
                        Shape::Rule(
                            Some(5 * Scaled::UNITY),
                            Some(6 * Scaled::UNITY),
                            Some(7 * Scaled::UNITY),
                        ),
                        Shape::Penalty(8),
                    ];
                    assert!(matches!(
                        register_shapes(&mut modes, 0).as_deref(),
                        Some([Shape::HBox { children, .. }]) if children.as_slice() == direct
                    ));
                    assert!(matches!(
                        register_shapes(&mut modes, 1).as_deref(),
                        Some([Shape::VBox { children, .. }])
                            if children.as_slice() == [
                                Shape::Kern(-Scaled::UNITY, KernKind::Explicit),
                                Shape::Glue {
                                    width: 2 * Scaled::UNITY,
                                    stretch: 3 * Scaled::UNITY,
                                    stretch_order: Order::Fill,
                                    shrink: 4 * Scaled::UNITY,
                                    shrink_order: Order::Fil,
                                    kind: GlueKind::Normal,
                                    leader: false,
                                },
                                Shape::Rule(Some(5 * Scaled::UNITY), Some(6 * Scaled::UNITY), Some(7 * Scaled::UNITY)),
                                Shape::Penalty(8),
                            ]
                    ));
                    assert!(
                        matches!(
                            register_shapes(&mut modes, 2).as_deref(),
                            Some([Shape::HBox { children, .. }])
                                if children.as_slice() == [
                                    Shape::MathOn(0),
                                    direct[0].clone(),
                                    direct[1].clone(),
                                    direct[2].clone(),
                                    direct[3].clone(),
                                    Shape::MathOff(0),
                                ]
                        ),
                        "{:?}; terminal={}",
                        register_shapes(&mut modes, 2),
                        terminal(&modes)
                    );

                    with_run(
                        br"\everypar{\global\advance\count0 by1}
          \vskip1pt\xdef\aftervskip{\the\count0}
          \hrule height1pt\xdef\afterhrule{\the\count0}
          \kern1pt\penalty0\xdef\afterkern{\the\count0}
          \hskip1pt\par\xdef\afterhskip{\the\count0}
          \vrule width1pt\par\xdef\aftervrule{\the\count0}",
                        false,
                        |control, outer| {
                            assert_eq!(control.current_mode(), crate::mode::Mode::Vertical);
                            assert_eq!(macro_text(&mut outer, "aftervskip"), "0");
                            assert_eq!(macro_text(&mut outer, "afterhrule"), "0");
                            assert_eq!(macro_text(&mut outer, "afterkern"), "0");
                            assert_eq!(macro_text(&mut outer, "afterhskip"), "1");
                            assert_eq!(macro_text(&mut outer, "aftervrule"), "2");
                        },
                    );
                },
            );
        },
    );
}

#[test]
fn direct_material_math_recovery_and_failed_keyword_token_ownership_matrix() {
    // TeX82 §§1046--1047/1055--1062: math-mode `\hrule` and `\vskip`
    // recover by inserting a math shift before either command scans an
    // operand. A failed rule keyword backs up the exact offending token;
    // nullfont recovery then proves that token executes once before the
    // following kern, rather than being swallowed by rule scanning.
    with_run(
        br"\nonstopmode\setbox0=\hbox{$\hrule\kern2pt}",
        false,
        |_, mut hrule| {
            assert_eq!(
                register_shapes(&mut hrule, 0),
                Some(vec![Shape::HBox {
                    width: 2 * Scaled::UNITY,
                    height: 0,
                    depth: 0,
                    shift: 0,
                    children: vec![
                        Shape::MathOn(0),
                        Shape::MathOff(0),
                        Shape::Kern(2 * Scaled::UNITY, KernKind::Explicit),
                    ],
                }])
            );
            let hrule_terminal = terminal(&hrule);
            let missing_shift = hrule_terminal
                .find("Missing $ inserted")
                .expect("math recovery");
            let restricted_rule = hrule_terminal
                .find("You can't use `\\hrule' here except with leaders")
                .expect("restricted-horizontal rule rejection");
            assert!(missing_shift < restricted_rule, "{hrule_terminal}");

            with_run(
                br"\nonstopmode\noindent$\vskip1pt\global\count0=7\par",
                false,
                |control, vskip| {
                    assert_eq!(count_register(&mut vskip, 0), 7);
                    assert_eq!(control.current_mode(), crate::mode::Mode::Vertical);
                    assert_eq!(terminal(&vskip).matches("Missing $ inserted").count(), 1);

                    let source =
        br"\nonstopmode\tracinglostchars=1\nullfont\setbox0=\hbox{\vrule width1pt X\kern2pt}";
                    with_run_observed(source, false, |_, mut universe, observations| {
                        assert_eq!(
                            register_shapes(&mut universe, 0),
                            Some(vec![Shape::HBox {
                                width: 3 * Scaled::UNITY,
                                height: 0,
                                depth: 0,
                                shift: 0,
                                children: vec![
                                    Shape::Rule(Some(Scaled::UNITY), None, None),
                                    Shape::Kern(2 * Scaled::UNITY, KernKind::Explicit),
                                ],
                            }])
                        );
                        assert_eq!(
                            terminal(&universe)
                                .matches("Missing character: There is no X")
                                .count(),
                            1
                        );
                        let x = ObservedToken::Character {
                            character: 'X',
                            catcode: tex_state::token::Catcode::Letter,
                        };
                        let recoveries: Vec<_> = observations
                            .0
                            .iter()
                            .filter_map(|event| match event {
                                CommandObservation::Recovery(recovery)
                                    if recovery.kind == RecoveryKind::Backup
                                        && recovery.tokens == [x.clone()] =>
                                {
                                    Some((recovery.kind, recovery.tokens.clone()))
                                }
                                _ => None,
                            })
                            .collect();
                        assert_eq!(
                            recoveries,
                            [
                                (RecoveryKind::Backup, vec![x.clone()]),
                                (RecoveryKind::Backup, vec![x.clone()]),
                                (RecoveryKind::Backup, vec![x.clone()]),
                            ]
                        );
                        let deliveries: Vec<_> = observations
                            .0
                            .iter()
                            .filter_map(|event| match event {
                                CommandObservation::Command(command)
                                    if command.spelling == x
                                        || command.spelling
                                            == ObservedToken::ControlSequence("kern".into()) =>
                                {
                                    Some((
                                        command.boundary,
                                        command.spelling.clone(),
                                        command.command.clone(),
                                        command
                                            .provenance
                                            .source_range
                                            .map(|range| (range.start(), range.end())),
                                    ))
                                }
                                _ => None,
                            })
                            .collect();
                        let raw_x = |range| {
                            (
                                CommandDeliveryBoundary::Raw,
                                x.clone(),
                                "letter".into(),
                                range,
                            )
                        };
                        let expanded_x = |range| {
                            (
                                CommandDeliveryBoundary::Expanded,
                                x.clone(),
                                "letter".into(),
                                range,
                            )
                        };
                        assert_eq!(
                            deliveries,
                            vec![
                                raw_x(Some((71, 72))),
                                expanded_x(Some((71, 72))),
                                raw_x(None),
                                expanded_x(None),
                                raw_x(None),
                                expanded_x(None),
                                raw_x(None),
                                expanded_x(None),
                                (
                                    CommandDeliveryBoundary::Raw,
                                    ObservedToken::ControlSequence("kern".into()),
                                    "kern".into(),
                                    Some((72, 77)),
                                ),
                                (
                                    CommandDeliveryBoundary::Expanded,
                                    ObservedToken::ControlSequence("kern".into()),
                                    "kern".into(),
                                    Some((72, 77)),
                                ),
                            ]
                        );
                    });
                },
            );
        },
    );
}

#[test]
fn box_construction_targets_specs_hooks_shifts_leaders_and_register_matrix() {
    // TeX82 §§1071--1087: one ordered matrix spans constructors/specs,
    // everybox hooks, local/global targets, shifts, leaders, shipout, copy,
    // take, lastbox, vtop adjustment, and scanner recovery.
    with_run(
        br"\everyhbox{\global\advance\count0 by1}\everyvbox{\global\advance\count1 by1}
          \setbox0=\hbox{\kern1pt}
          \setbox1=\hbox to10pt{\hfil}
          \setbox2=\hbox spread2pt{\hfil}
          \setbox3=\vbox{\hrule height3pt}
          \setbox4=\vtop{\hrule height3pt\kern2pt\hrule height4pt}
          {\global\setbox5=\hbox{}\setbox6=\hbox{}}
          \setbox7=\copy0 \setbox8=\box0
          \setbox9=\hbox{\hbox{\kern4pt}\global\setbox10=\lastbox}
          \setbox11=\hbox{\raise2pt\hbox{\kern1pt}\leaders\hbox{\kern1pt}\hskip6pt}
          \setbox12=\vbox{\moveright3pt\vbox{\kern1pt}}
          \shipout\hbox{}\end",
        false,
        |_, mut universe| {
            assert_eq!(
                count_register(&mut universe, 0),
                11,
                "all eleven hbox constructors run everyhbox"
            );
            assert_eq!(
                count_register(&mut universe, 1),
                4,
                "four vbox/vtop constructors run everyvbox"
            );
            assert_eq!(register_box_width(&mut universe, 1), 10 * Scaled::UNITY);
            assert_eq!(register_box_width(&mut universe, 2), 2 * Scaled::UNITY);
            assert_eq!(register_box_height(&mut universe, 4), 3 * Scaled::UNITY);
            assert!(register_shapes(&mut universe, 5).is_some());
            assert_eq!(register_shapes(&mut universe, 6), None);
            assert_eq!(register_shapes(&mut universe, 0), None);
            assert_eq!(
                register_shapes(&mut universe, 7),
                register_shapes(&mut universe, 8)
            );
            assert!(
                matches!(register_shapes(&mut universe, 9).as_deref(), Some([Shape::HBox { children, .. }]) if children.is_empty())
            );
            assert!(
                matches!(register_shapes(&mut universe, 10).as_deref(), Some([Shape::HBox { children, .. }]) if matches!(children.as_slice(), [Shape::Kern(k, KernKind::Explicit)] if *k == 4 * Scaled::UNITY))
            );
            assert!(
                matches!(register_shapes(&mut universe, 11).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { shift, .. }, Shape::Glue { leader: true, .. }] if *shift == -2 * Scaled::UNITY))
            );
            assert!(
                matches!(register_shapes(&mut universe, 12).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::VBox { shift, .. }] if *shift == 3 * Scaled::UNITY))
            );
            assert_eq!(universe.world().artifact_commits().len(), 1);

            with_run(
                br"\nonstopmode\setbox0=\hbox\kern2pt}\setbox1=\count0=7\setbox2=\hbox{}",
                false,
                |_, mut recovery| {
                    assert_eq!(count_register(&mut recovery, 0), 7);
                    assert!(register_shapes(&mut recovery, 2).is_some());
                    assert!(terminal(&recovery).contains("Missing { inserted"));
                    assert!(terminal(&recovery).contains("Improper \\setbox"));
                },
            );
        },
    );
}

#[test]
fn paragraph_entry_endings_migration_depth_and_recovery_matrix() {
    // TeX82 §§1088--1096: explicit/implicit entry, indent ordering, empty and
    // discardable endings, vertical-trigger and group-close endings,
    // migration, internal-v versus outer-page contribution, and recovery are
    // distinguished by their exact nested node sequences.
    with_run(
        br"\font\f=cmr10 \f \hsize=100pt \everypar{\global\advance\count0 by1\kern1pt}
          \setbox0=\vbox{\indent A\par}
          \setbox1=\vbox{\noindent B\par}
          \setbox2=\vbox{C\par}
          \setbox3=\vbox{\noindent\hskip1pt\par}
          \setbox4=\vbox{\noindent D\mark{m}\vadjust{\kern2pt}\vskip3pt}
          \setbox5=\vbox{{\noindent E}}
          \noindent F\par\end",
        true,
        |_, mut universe| {
            assert_eq!(count_register(&mut universe, 0), 7);
            let indented = register_shapes(&mut universe, 0);
            assert!(
                matches!(indented.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if matches!(line.as_slice(), [Shape::HBox { children: indent, .. }, Shape::Kern(k, KernKind::Explicit), Shape::Char('A'), Shape::Penalty(10_000), Shape::Glue { kind: GlueKind::ParFillSkip, leader: false, .. }, Shape::Glue { kind: GlueKind::RightSkip, leader: false, .. }] if indent.is_empty() && *k == Scaled::UNITY))),
                "{indented:?}"
            );
            let noindent = register_shapes(&mut universe, 1);
            assert!(
                matches!(noindent.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if matches!(line.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Char('B'), Shape::Penalty(10_000), Shape::Glue { kind: GlueKind::ParFillSkip, leader: false, .. }, Shape::Glue { kind: GlueKind::RightSkip, leader: false, .. }] if *k == Scaled::UNITY))),
                "{noindent:?}"
            );
            let implicit = register_shapes(&mut universe, 2);
            assert!(
                matches!(implicit.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if matches!(line.as_slice(), [Shape::HBox { children: indent, .. }, Shape::Kern(k, KernKind::Explicit), Shape::Char('C'), Shape::Penalty(10_000), Shape::Glue { kind: GlueKind::ParFillSkip, leader: false, .. }, Shape::Glue { kind: GlueKind::RightSkip, leader: false, .. }] if indent.is_empty() && *k == Scaled::UNITY))),
                "{implicit:?}"
            );
            let discardable = register_shapes(&mut universe, 3);
            assert!(
                matches!(discardable.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if matches!(line.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Penalty(10_000), Shape::Glue { width: 0, kind: GlueKind::ParFillSkip, leader: false, .. }, Shape::Glue { width: 0, kind: GlueKind::RightSkip, leader: false, .. }] if *k == Scaled::UNITY))),
                "{discardable:?}"
            );
            assert!(
                matches!(register_shapes(&mut universe, 4).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { .. }, Shape::Mark(0, mark), Shape::Kern(k, KernKind::Explicit), Shape::Glue { width, leader: false, .. }] if mark == "m" && *k == 2 * Scaled::UNITY && *width == 3 * Scaled::UNITY))
            );
            assert!(
                matches!(register_shapes(&mut universe, 5).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }] if line.contains(&Shape::Char('E'))))
            );
            assert_eq!(universe.world().artifact_commits().len(), 1);
            assert!(!terminal(&universe).contains("Missing \\par inserted"));
            assert!(!terminal(&universe).contains("Emergency stop"));
        },
    );
}

#[test]
fn structured_material_legal_mode_and_source_order_matrix() {
    // TeX82 §§1097--1103: insert, mark, and penalty are legal in all
    // three outer mode classes.  Project their actual nodes in source order;
    // a diagnostic count or `lastnodetype` enquiry would not prove ownership.
    with_run(
        br"\vsize=1000pt\insert2{\kern2pt}\mark{outer}\penalty10000",
        false,
        |_, outer| {
            let outer_nodes = outer_vertical_shapes(&mut outer);
            assert!(
                matches!(outer_nodes.as_slice(), [Shape::Insert { class: 2, content, .. }, Shape::Mark(0, mark)]
            if content == &[Shape::Kern(2 * Scaled::UNITY, KernKind::Explicit)] && mark == "outer"),
                "{outer_nodes:?}"
            );
            assert_eq!(page_last_penalty(&mut outer), 10_000);

            with_run(
                br"\setbox0=\hbox{\insert3{\kern3pt}\mark{horizontal}\penalty51}
          \setbox1=\vbox{\insert4{\kern4pt}\mark{vertical}\penalty52}",
                false,
                |_, mut nested| {
                    let horizontal = register_shapes(&mut nested, 0);
                    assert!(
                        matches!(horizontal.as_deref(), Some([Shape::HBox { children, .. }])
            if matches!(children.as_slice(), [Shape::Insert { class: 3, content, .. }, Shape::Mark(0, mark), Shape::Penalty(51)]
                if content == &[Shape::Kern(3 * Scaled::UNITY, KernKind::Explicit)] && mark == "horizontal")),
                        "{horizontal:?}"
                    );
                    let vertical = register_shapes(&mut nested, 1);
                    assert!(
                        matches!(vertical.as_deref(), Some([Shape::VBox { children, .. }])
            if matches!(children.as_slice(), [Shape::Insert { class: 4, content, .. }, Shape::Mark(0, mark), Shape::Penalty(52)]
                if content == &[Shape::Kern(4 * Scaled::UNITY, KernKind::Explicit)] && mark == "vertical")),
                        "{vertical:?}"
                    );
                    with_run_until_count(
                        br"\noindent$\insert5{\kern5pt}\mark{math}\penalty53\global\count0=1",
                        1,
                        |math_control, math_stores| {
                            let math =
                                shapes(&mut math_stores, math_control.modes.current_list().nodes());
                            assert!(
                                matches!(math.as_slice(), [Shape::Insert { class: 5, content, .. }, Shape::Mark(0, mark), Shape::Penalty(53)]
            if content == &[Shape::Kern(5 * Scaled::UNITY, KernKind::Explicit)] && mark == "math"),
                                "{math:?}"
                            );
                        },
                    );
                },
            );
        },
    );
}

#[test]
fn insert_closure_snapshots_parameters_and_migrates_owned_nodes_in_order() {
    // TeX82 §§1100 reads all three insertion parameters before unsave.
    // Both hbox contribution and paragraph line-breaking then move the one
    // insertion node, mark node, and vadjust content to the enclosing vlist.
    with_run(
        br"\font\f=cmr10 \f\splittopskip=1pt\splitmaxdepth=2pt\floatingpenalty=3
          \setbox0=\vbox{\hbox{A\insert7{\splittopskip=11pt\splitmaxdepth=12pt\floatingpenalty=13\kern2pt}\mark{boxed}\vadjust{\kern3pt}B}}
          \setbox1=\vbox{\noindent C\insert8{\splittopskip=21pt\splitmaxdepth=22pt\floatingpenalty=23\kern4pt}\mark{paragraph}\vadjust{\kern5pt}D\par}",
        true, |_, mut universe| {
    assert_eq!(
        glue_parameter(&mut universe, GlueParam::SPLIT_TOP_SKIP)
            .width
            .raw(),
        Scaled::UNITY
    );
    assert_eq!(
        dimen_parameter(&mut universe, DimenParam::SPLIT_MAX_DEPTH).raw(),
        2 * Scaled::UNITY
    );
    assert_eq!(int_parameter(&mut universe, IntParam::FLOATING_PENALTY), 3);

    let boxed = register_shapes(&mut universe, 0);
    assert!(
        matches!(boxed.as_deref(), Some([Shape::VBox { children, .. }])
            if matches!(children.as_slice(), [Shape::HBox { children: retained, .. }, Shape::Insert { class: 7, size, split_top_skip, split_max_depth, floating_penalty: 13, content, .. }, Shape::Mark(0, mark), Shape::Kern(adjust, KernKind::Explicit)]
                if retained == &[Shape::Char('A'), Shape::Char('B')]
                    && *size == 2 * Scaled::UNITY
                    && *split_top_skip == 11 * Scaled::UNITY
                    && *split_max_depth == 12 * Scaled::UNITY
                    && content == &[Shape::Kern(2 * Scaled::UNITY, KernKind::Explicit)]
                    && mark == "boxed"
                    && *adjust == 3 * Scaled::UNITY)),
        "{boxed:?}"
    );
    let paragraph = register_shapes(&mut universe, 1);
    assert!(
        matches!(paragraph.as_deref(), Some([Shape::VBox { children, .. }])
            if matches!(children.as_slice(), [Shape::HBox { children: retained, .. }, Shape::Insert { class: 8, size, split_top_skip, split_max_depth, floating_penalty: 23, content, .. }, Shape::Mark(0, mark), Shape::Kern(adjust, KernKind::Explicit)]
                if retained.contains(&Shape::Char('C')) && retained.contains(&Shape::Char('D'))
                    && *size == 4 * Scaled::UNITY
                    && *split_top_skip == 21 * Scaled::UNITY
                    && *split_max_depth == 22 * Scaled::UNITY
                    && content == &[Shape::Kern(4 * Scaled::UNITY, KernKind::Explicit)]
                    && mark == "paragraph"
                    && *adjust == 5 * Scaled::UNITY)),
        "{paragraph:?}"
    );

    });
}

#[test]
fn unbox_copy_move_void_wrong_kind_and_math_ownership_matrix() {
    // TeX82 §§1110: copy and move splice the same ordered child list,
    // but only move voids the source. Void registers are silent no-ops;
    // wrong-kind and math-mode attempts diagnose without changing ownership.
    with_run(
        br"\nonstopmode
          \setbox0=\vbox{\hrule height1pt\kern2pt\penalty3}
          \setbox1=\vbox{\unvcopy0\kern4pt\unvbox0}
          \setbox2=\hbox{\kern5pt}
          \setbox3=\vbox{\unvbox9\global\advance\count0 by1
                           \unvcopy9\global\advance\count0 by1
                           \unvbox2\global\advance\count0 by1
                           \unvcopy2\global\advance\count0 by1}
          \setbox4=\vbox{\kern6pt}
          \setbox5=\hbox{\unhbox9\global\advance\count0 by1
                           \unhcopy9\global\advance\count0 by1
                           \unhbox4\global\advance\count0 by1
                           \unhcopy4\global\advance\count0 by1}
          \setbox6=\hbox{\kern7pt}
          \setbox7=\hbox{$\unhbox6\global\advance\count0 by1
                            \unhcopy6\global\advance\count0 by1$}
          \setbox8=\hbox{\kern8pt\hskip9pt\penalty10}
          \setbox9=\hbox{\unhcopy8\kern11pt\global\setbox10=\copy8\unhbox8}",
        false,
        |_, mut universe| {
            let moved = register_shapes(&mut universe, 1);
            assert!(
                matches!(moved.as_deref(), Some([Shape::VBox { children, .. }])
                if children.as_slice() == [
                    Shape::Rule(None, Some(Scaled::UNITY), Some(0)),
                    Shape::Kern(2 * Scaled::UNITY, KernKind::Explicit),
                    Shape::Penalty(3),
                    Shape::Kern(4 * Scaled::UNITY, KernKind::Explicit),
                    Shape::Rule(None, Some(Scaled::UNITY), Some(0)),
                    Shape::Kern(2 * Scaled::UNITY, KernKind::Explicit),
                    Shape::Penalty(3),
                ]),
                "{moved:?}"
            );
            assert_eq!(register_shapes(&mut universe, 0), None, "unvbox moves");
            let horizontal = register_shapes(&mut universe, 9);
            assert!(
                matches!(horizontal.as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [
            Shape::Kern(first, KernKind::Explicit),
            Shape::Glue { width: first_glue, kind: GlueKind::Normal, leader: false, .. },
            Shape::Penalty(10),
            Shape::Kern(copied_kern, KernKind::Explicit),
            Shape::Kern(second, KernKind::Explicit),
            Shape::Glue { width: second_glue, kind: GlueKind::Normal, leader: false, .. },
            Shape::Penalty(10),
        ] if *first == 8 * Scaled::UNITY
            && *first_glue == 9 * Scaled::UNITY
            && *copied_kern == 11 * Scaled::UNITY
            && *second == 8 * Scaled::UNITY
            && *second_glue == 9 * Scaled::UNITY)),
                "{horizontal:?}"
            );
            assert!(
                matches!(register_shapes(&mut universe, 10).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [
            Shape::Kern(kern, KernKind::Explicit),
            Shape::Glue { width, kind: GlueKind::Normal, leader: false, .. },
            Shape::Penalty(10),
        ] if *kern == 8 * Scaled::UNITY && *width == 9 * Scaled::UNITY)),
                "unhcopy preserves the source before the matching move"
            );
            assert_eq!(register_shapes(&mut universe, 8), None, "unhbox moves");
            assert!(
                register_shapes(&mut universe, 2).is_some(),
                "wrong v-unbox preserves hbox"
            );
            assert!(
                register_shapes(&mut universe, 4).is_some(),
                "wrong h-unbox preserves vbox"
            );
            assert!(
                register_shapes(&mut universe, 6).is_some(),
                "math unbox preserves hbox"
            );
            assert!(
                matches!(register_shapes(&mut universe, 3).as_deref(), Some([Shape::VBox { children, .. }]) if children.is_empty())
            );
            assert!(
                matches!(register_shapes(&mut universe, 5).as_deref(), Some([Shape::HBox { children, .. }]) if children.is_empty())
            );
            assert!(
                matches!(register_shapes(&mut universe, 7).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::MathOn(0), Shape::MathOff(0)])
            );
            assert_eq!(
                count_register(&mut universe, 0),
                10,
                "all following assignments execute"
            );
            assert_eq!(
                terminal(&universe)
                    .matches("Incompatible list can't be unboxed")
                    .count(),
                6
            );
        },
    );
}

#[test]
fn delete_last_matches_only_the_live_tail_in_h_v_and_math_modes() {
    // TeX82 §§1105: each primitive removes only its own live tail.
    // Empty and mismatched operations are exact structural no-ops.
    with_run(
        br"\font\f=cmr10 \f
          \setbox0=\hbox{\unkern\kern1pt\unkern\kern2pt\unskip
                           \hskip3pt\unskip\penalty4\unpenalty A\unpenalty}
          \setbox1=\vbox{\unskip\kern5pt\unkern\kern6pt\unpenalty
                           \vskip7pt\unskip\penalty8\unpenalty\hrule\unkern}
          ",
        true,
        |_, mut universe| {
            assert!(
                matches!(register_shapes(&mut universe, 0).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::Kern(2 * Scaled::UNITY, KernKind::Explicit), Shape::Char('A')])
            );
            assert!(
                matches!(register_shapes(&mut universe, 1).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Rule(_, _, _)] if *k == 6 * Scaled::UNITY))
            );
            with_run_until_count(
        br"\noindent$\unpenalty\kern9pt\unkern\kern10pt\unskip\mskip11mu\unskip\penalty12\unpenalty\global\count0=1",
        1, |math_control, math_stores| {
    let math = shapes(&mut math_stores, math_control.modes.current_list().nodes());
    assert!(
        math.as_slice() == [Shape::Kern(10 * Scaled::UNITY, KernKind::Explicit)],
        "{math:?}"
    );

    });
        },
    );
}

#[test]
fn outer_vertical_delete_recovery_preserves_page_and_following_input() {
    // Once build_page has consumed the contribution tail, unpenalty and
    // unkern take §1105's apology path. Unskip uniquely stays silent after a
    // nonglue and removes a still-pending matching contribution tail.
    for (baseline_source, source, command) in [
        (
            br"\nonstopmode\hrule height1pt\penalty10000".as_slice(),
            br"\nonstopmode\hrule height1pt\penalty10000\unpenalty\global\count0=11".as_slice(),
            "unpenalty",
        ),
        (
            br"\nonstopmode\hrule height1pt\penalty10000".as_slice(),
            br"\nonstopmode\hrule height1pt\penalty10000\unkern\global\count0=11".as_slice(),
            "unkern",
        ),
    ] {
        with_run(baseline_source, false, |_, baseline| {
            with_run(source, false, |_, mut universe| {
                assert_eq!(
                    count_register(&mut universe, 0),
                    11,
                    "{command} lost following input"
                );
                assert!(
                    terminal(&universe)
                        .contains(&format!("You can't use `\\{command}' in vertical mode")),
                    "{command}: {}",
                    terminal(&universe)
                );
                assert_eq!(
                    outer_vertical_shapes(&mut universe),
                    outer_vertical_shapes(&mut baseline),
                    "{command} changed page ownership"
                );
            });
        });
    }

    with_run(
        br"\hrule height1pt\penalty10000",
        false,
        |_, nonglue_baseline| {
            with_run(
                br"\nonstopmode\hrule height1pt\penalty10000\unskip\global\count0=11",
                false,
                |_, nonglue| {
                    assert_eq!(count_register(&mut nonglue, 0), 11);
                    assert_eq!(
                        outer_vertical_shapes(&mut nonglue),
                        outer_vertical_shapes(&mut nonglue_baseline)
                    );
                    assert!(!terminal(&nonglue).contains("You can't use `\\unskip'"));

                    with_run(br"\hrule height1pt", false, |_, matching_baseline| {
                        with_run(
                            br"\hrule height1pt\vskip2pt\unskip\global\count0=11",
                            false,
                            |_, matching| {
                                assert_eq!(count_register(&mut matching, 0), 11);
                                assert_eq!(
                                    outer_vertical_shapes(&mut matching),
                                    outer_vertical_shapes(&mut matching_baseline),
                                    "outer unskip removes only the matching contribution tail"
                                );
                            },
                        );
                    });
                },
            );
        },
    );
}

#[test]
fn italic_correction_uses_font_tail_math_zero_and_forbidden_recovery() {
    // TeX82 §§1112--1113: hmode consults only the immediately preceding
    // font character, math appends a zero font kern, and both vertical modes
    // diagnose without consuming the following assignment.
    with_run(
        br"\font\f=cmr10 \f\nonstopmode
          \setbox0=\hbox{f\/}
          \setbox1=\hbox{A\/}
          \setbox2=\hbox{f\kern1pt\/}
          \/\global\advance\count0 by1
          \setbox4=\vbox{\/\global\advance\count0 by1}",
        true,
        |_, mut universe| {
            assert!(
                matches!(register_shapes(&mut universe, 0).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Char('f'), Shape::Kern(amount, KernKind::Explicit)] if *amount > 0))
            );
            assert!(
                matches!(register_shapes(&mut universe, 1).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::Char('A'), Shape::Kern(0, KernKind::Explicit)])
            );
            assert!(
                matches!(register_shapes(&mut universe, 2).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::Char('f'), Shape::Kern(Scaled::UNITY, KernKind::Explicit)])
            );
            with_run_until_count(
                br"\noindent$\kern1pt\/\global\count0=1",
                1,
                |math_control, math_stores| {
                    let math = shapes(&mut math_stores, math_control.modes.current_list().nodes());
                    assert!(
                        math.as_slice()
                            == [
                                Shape::Kern(Scaled::UNITY, KernKind::Explicit),
                                Shape::Kern(0, KernKind::Font),
                            ],
                        "{math:?}"
                    );
                    assert_eq!(count_register(&mut universe, 0), 2);
                    assert!(outer_vertical_shapes(&mut universe).is_empty());
                    assert!(
                        matches!(register_shapes(&mut universe, 4).as_deref(), Some([Shape::VBox { children, .. }]) if children.is_empty())
                    );
                    assert_eq!(
                        terminal(&universe).matches("You can't use `\\/'").count(),
                        2
                    );
                },
            );
        },
    );
}

#[test]
fn insert_class_and_forbidden_vadjust_recovery_preserve_state_and_input() {
    // TeX82 §§1099/§1111: user class 255 and overflow recover to zero;
    // vadjust's internal 255 sentinel remains legal. Forbidden v/math paths
    // scan no body and leave its tokens plus all existing list state live.
    with_run(
        br"\nonstopmode
          \setbox0=\vbox{\insert0{\kern1pt}\insert254{\kern2pt}
                           \insert255{\kern3pt}\insert256{\kern4pt}
                           \vadjust\global\advance\count0 by1}
          \setbox1=\hbox{\kern5pt\vadjust{\kern6pt}\kern7pt}",
        false,
        |_, mut universe| {
            let classes = register_shapes(&mut universe, 0);
            assert!(
                matches!(classes.as_deref(), Some([Shape::VBox { children, .. }])
            if matches!(children.as_slice(), [
                Shape::Insert { class: 0, content: zero, .. },
                Shape::Insert { class: 254, content: high, .. },
                Shape::Insert { class: 0, content: reserved, .. },
                Shape::Insert { class: 0, content: overflow, .. },
            ] if zero == &[Shape::Kern(Scaled::UNITY, KernKind::Explicit)]
                && high == &[Shape::Kern(2 * Scaled::UNITY, KernKind::Explicit)]
                && reserved == &[Shape::Kern(3 * Scaled::UNITY, KernKind::Explicit)]
                && overflow == &[Shape::Kern(4 * Scaled::UNITY, KernKind::Explicit)])),
                "{classes:?}"
            );
            assert!(
                matches!(register_shapes(&mut universe, 1).as_deref(), Some([Shape::HBox { children, .. }])
                if children.as_slice() == [
                    Shape::Kern(5 * Scaled::UNITY, KernKind::Explicit),
                    Shape::Adjust(vec![Shape::Kern(6 * Scaled::UNITY, KernKind::Explicit)]),
                    Shape::Kern(7 * Scaled::UNITY, KernKind::Explicit),
                ])
            );
            with_run_until_count(
                br"\noindent$\vadjust{\kern8pt}\global\count0=1",
                1,
                |math_control, math_stores| {
                    let math = shapes(&mut math_stores, math_control.modes.current_list().nodes());
                    assert!(
                        math.as_slice()
                            == [Shape::Adjust(vec![Shape::Kern(
                                8 * Scaled::UNITY,
                                KernKind::Explicit,
                            )])],
                        "{math:?}"
                    );
                    assert_eq!(
                        count_register(&mut universe, 0),
                        1,
                        "forbidden vadjust consumes no following token"
                    );
                    let errors = terminal(&universe);
                    assert!(errors.contains("You can't \\insert255"), "{errors}");
                    assert!(errors.contains("Bad register code"), "{errors}");
                    assert_eq!(errors.matches("You can't use `\\vadjust'").count(), 1);

                    with_run(
                        br"\nonstopmode\vadjust\global\advance\count0 by1",
                        false,
                        |_, outer| {
                            assert_eq!(count_register(&mut outer, 0), 1);
                            assert!(outer_vertical_shapes(&mut outer).is_empty());
                            assert!(
                                terminal(&outer)
                                    .contains("You can't use `\\vadjust' in vertical mode")
                            );
                        },
                    );
                },
            );
        },
    );
}

#[test]
fn box_brace_hook_scope_and_aftergroup_order_matrix() {
    // TeX82 §§1074--1076/1085: brace aliases enter the same box group, each
    // nested construction runs its own hook after mode entry, and unsave
    // restores body-local state before releasing aftergroup material.
    with_run(
        br"\let\bgroup={\let\egroup=}
          \def\afterbox{\global\advance\count0 by1}
          \count0=2
          \everyhbox{\global\advance\count1 by1\kern1pt}
          \everyvbox{\global\advance\count6 by1\kern3pt}
          \setbox0=\hbox\bgroup
            \count0=10
            \aftergroup\afterbox
            \hbox\bgroup\kern2pt\egroup
          \egroup
          \setbox8=\vbox\bgroup\vbox\bgroup\kern4pt\egroup\egroup
          \global\multiply\count0 by10",
        false,
        |_, mut universe| {
            assert_eq!(
                count_register(&mut universe, 0),
                30,
                "local restoration precedes aftergroup"
            );
            assert_eq!(
                count_register(&mut universe, 1),
                2,
                "outer and nested hbox hooks run once"
            );
            assert_eq!(
                count_register(&mut universe, 6),
                2,
                "outer and nested vbox hooks run once"
            );
            assert!(
                matches!(register_shapes(&mut universe, 0).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Kern(outer, KernKind::Explicit), Shape::HBox { children: inner, .. }]
            if *outer == Scaled::UNITY
                && inner.as_slice() == [Shape::Kern(Scaled::UNITY, KernKind::Explicit), Shape::Kern(2 * Scaled::UNITY, KernKind::Explicit)])),
                "{:?}",
                register_shapes(&mut universe, 0)
            );
            assert!(
                matches!(register_shapes(&mut universe, 8).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Kern(outer, KernKind::Explicit), Shape::VBox { children: inner, .. }]
            if *outer == 3 * Scaled::UNITY
                && inner.as_slice() == [Shape::Kern(3 * Scaled::UNITY, KernKind::Explicit), Shape::Kern(4 * Scaled::UNITY, KernKind::Explicit)])),
                "{:?}",
                register_shapes(&mut universe, 8)
            );
        },
    );
}

#[test]
fn box_void_vtop_box255_and_zero_shift_matrix() {
    // TeX82 §§1077--1087: void copy/take operands stay void, box255 obeys
    // the same destructive distinction, vtop's first-item rule is exact at
    // empty and leading-glue boundaries, and zero shifts remain typed boxes.
    with_run(br"\setbox255=\hbox{\kern1pt}", false, |_, mut original| {
        let expected_box255 = vec![Shape::HBox {
            width: Scaled::UNITY,
            height: 0,
            depth: 0,
            shift: 0,
            children: vec![Shape::Kern(Scaled::UNITY, KernKind::Explicit)],
        }];
        assert_eq!(
            register_shapes(&mut original, 255),
            Some(expected_box255.clone()),
            "box255 starts as the exact non-void 1pt hbox"
        );

        with_run(
            br"\setbox0=\copy7\setbox1=\box7
          \setbox255=\hbox{\kern1pt}\setbox2=\copy255\setbox3=\box255
          \setbox4=\vtop{}
          \setbox5=\vtop{\vskip2pt\hrule height3pt}
          \setbox6=\hbox{\raise0pt\hbox{\kern4pt}\lower0pt\vbox{\kern5pt}}
          \setbox7=\vbox{\moveleft0pt\hbox{\kern6pt}\moveright0pt\vbox{\kern7pt}}",
            false,
            |_, mut universe| {
                assert_eq!(register_shapes(&mut universe, 0), None);
                assert_eq!(register_shapes(&mut universe, 1), None);
                let copied =
                    register_shapes(&mut universe, 2).expect("copy255 destination is non-void");
                let moved =
                    register_shapes(&mut universe, 3).expect("box255 destination is non-void");
                assert_eq!(copied, expected_box255);
                assert_eq!(moved, copied);
                assert_eq!(register_shapes(&mut universe, 255), None);
                assert_eq!(register_box_height(&mut universe, 4), 0);
                assert_eq!(register_box_depth(&mut universe, 4), 0);
                assert_eq!(register_box_height(&mut universe, 5), 0);
                assert_eq!(register_box_depth(&mut universe, 5), 5 * Scaled::UNITY);
                assert!(
                    matches!(register_shapes(&mut universe, 6).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { shift: 0, children: raised, .. }, Shape::VBox { shift: 0, children: lowered, .. }]
            if raised.as_slice() == [Shape::Kern(4 * Scaled::UNITY, KernKind::Explicit)]
                && lowered.as_slice() == [Shape::Kern(5 * Scaled::UNITY, KernKind::Explicit)]))
                );
                assert!(
                    matches!(register_shapes(&mut universe, 7).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { shift: 0, children: left, .. }, Shape::Glue { leader: false, .. }, Shape::VBox { shift: 0, children: right, .. }]
            if left.as_slice() == [Shape::Kern(6 * Scaled::UNITY, KernKind::Explicit)]
                && right.as_slice() == [Shape::Kern(7 * Scaled::UNITY, KernKind::Explicit)]))
                );
            },
        );
    });
}

#[test]
fn insert_and_vadjust_aftergroup_closure_provenance_order_and_once() {
    // TeX82 §§282/1098: insertion-group unsave backs up each aftergroup token
    // with its inserted provenance before the following source token. Exact
    // character order proves the replay happens once for both insert and
    // vadjust, while character nodes retain the direct token origin.
    with_run(
        br"\font\f=cmr10 \f \hsize=100pt
          \setbox8=\vbox{\insert1{\kern1pt\aftergroup A}B\par}
          \setbox9=\vbox{\noindent A\vadjust{\kern4pt\aftergroup B}C\par}",
        true,
        |_, mut universe| {
            let insert_box = register_box(&mut universe, 8);
            let insert_children = page_vec(&mut universe, insert_box.children);
            let [
                Node::Ins { content, .. },
                Node::Glue {
                    kind: GlueKind::ParSkip,
                    ..
                },
                Node::HList(insert_line),
            ] = insert_children.as_slice()
            else {
                panic!(
                    "insert aftergroup closure: {:?}",
                    shapes(&mut universe, &insert_children)
                );
            };
            let insertion_content = page_vec(&mut universe, *content);
            assert_eq!(
                shapes(&mut universe, &insertion_content),
                vec![Shape::Kern(Scaled::UNITY, KernKind::Explicit)]
            );
            let insert_chars = page_vec(&mut universe, insert_line.children)
                .iter()
                .filter_map(|node| match node {
                    Node::Char { ch, origin, .. } => Some((*ch, origin.clone())),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                insert_chars.iter().map(|(ch, _)| *ch).collect::<String>(),
                "AB"
            );
            assert!(
                universe.origin_is_inserted_kind(
                    insert_chars[0].1.id(),
                    InsertedOriginKind::AfterGroup
                )
            );

            let adjust_box = register_box(&mut universe, 9);
            let adjust_children = page_vec(&mut universe, adjust_box.children);
            let [
                Node::HList(line),
                Node::Kern {
                    amount,
                    kind: KernKind::Explicit,
                },
            ] = adjust_children.as_slice()
            else {
                panic!(
                    "vadjust paragraph closure: {:?}",
                    shapes(&mut universe, &adjust_children)
                );
            };
            assert_eq!(*amount, Scaled::from_raw(4 * Scaled::UNITY));
            let line_chars = page_vec(&mut universe, line.children)
                .iter()
                .filter_map(|node| match node {
                    Node::Char { ch, origin, .. } => Some((*ch, origin.clone())),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                line_chars.iter().map(|(ch, _)| *ch).collect::<String>(),
                "ABC",
                "aftergroup token precedes the following source token"
            );
            assert!(
                universe
                    .origin_is_inserted_kind(line_chars[1].1.id(), InsertedOriginKind::AfterGroup)
            );
        },
    );
}

#[test]
fn box_forbidden_shift_lastbox_vsplit_and_recovery_ownership_matrix() {
    // TeX82 §§1079--1084: forbidden complementary shifts do not scan an
    // operand, outer-page lastbox is void, and invalid vsplit preserves its
    // source register. Missing opener/operand recovery replays the rejected
    // token exactly once, so body and following assignments remain owned.
    with_run(
        br"\nonstopmode
          \raise\global\count0=11
          \noindent\moveright\global\count1=12\par
          \setbox0=\lastbox\global\count2=13
          \setbox3=\hbox{\kern7pt}\setbox4=\vsplit3 to5pt
          \global\count3=14",
        false,
        |_, mut forbidden| {
            assert_eq!(
                (
                    count_register(&mut forbidden, 0),
                    count_register(&mut forbidden, 1)
                ),
                (11, 12)
            );
            assert_eq!(
                (
                    count_register(&mut forbidden, 2),
                    count_register(&mut forbidden, 3)
                ),
                (13, 14)
            );
            assert_eq!(register_shapes(&mut forbidden, 0), None);
            assert!(matches!(
                register_shapes(&mut forbidden, 3).as_deref(),
                Some([Shape::HBox { children, .. }])
                    if children.as_slice() == [Shape::Kern(7 * Scaled::UNITY, KernKind::Explicit)]
            ));
            assert_eq!(register_shapes(&mut forbidden, 4), None);
            let errors = terminal(&forbidden);
            assert_eq!(errors.matches("You can't use").count(), 3, "{errors}");
            assert!(errors.contains("\\vsplit needs a \\vbox"), "{errors}");

            with_run(
                br"\nonstopmode
          \setbox5=\hbox\kern2pt}
          \setbox6=\global\count4=15
          \setbox7=\hbox{\kern3pt}",
                false,
                |_, mut recovery| {
                    assert!(matches!(
                        register_shapes(&mut recovery, 5).as_deref(),
                        Some([Shape::HBox { children, .. }])
                            if children.as_slice() == [Shape::Kern(2 * Scaled::UNITY, KernKind::Explicit)]
                    ));
                    assert_eq!(count_register(&mut recovery, 4), 15);
                    assert!(matches!(
                        register_shapes(&mut recovery, 7).as_deref(),
                        Some([Shape::HBox { children, .. }])
                            if children.as_slice() == [Shape::Kern(3 * Scaled::UNITY, KernKind::Explicit)]
                    ));
                    let errors = terminal(&recovery);
                    assert_eq!(errors.matches("Missing { inserted").count(), 1, "{errors}");
                    assert_eq!(errors.matches("Improper \\setbox").count(), 1, "{errors}");
                },
            );
        },
    );
}

#[test]
fn paragraph_empty_discardable_display_and_insert_matrix() {
    // TeX82 §§1088--1096: a genuinely null noindent paragraph contributes no
    // line, a discardable-only nonnull list follows line breaking, display
    // entry ends the preceding paragraph, and insert material migrates after
    // the line rather than remaining among its children.
    with_run(
        br"\font\f=cmr10 \font\sy=cmsy10 \font\ex=cmex10
          \textfont2=\sy\scriptfont2=\sy\scriptscriptfont2=\sy
          \textfont3=\ex\scriptfont3=\ex\scriptscriptfont3=\ex
          \f \hsize=100pt
          \setbox0=\vbox{\noindent\par}
          \setbox1=\vbox{\noindent\hskip1pt\kern2pt\penalty7\par}
          \setbox2=\vbox{\noindent A$$$$}
          \setbox3=\vbox{\noindent B\insert4{\kern5pt}C\par}",
        true,
        |_, mut universe| {
            assert!(
                matches!(register_shapes(&mut universe, 0).as_deref(), Some([Shape::VBox { children, .. }]) if children.is_empty()),
                "{:?}",
                register_shapes(&mut universe, 0)
            );
            assert!(
                matches!(register_shapes(&mut universe, 1).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if matches!(line.as_slice(), [
                Shape::Glue { width: left, kind: GlueKind::Normal, leader: false, .. },
                Shape::Kern(kern, KernKind::Explicit),
                Shape::Penalty(7),
                Shape::Penalty(10_000),
                Shape::Glue { width: par_fill, kind: GlueKind::ParFillSkip, leader: false, .. },
                Shape::Glue { width: right, kind: GlueKind::RightSkip, leader: false, .. },
            ] if *left == Scaled::UNITY && *kern == 2 * Scaled::UNITY && *par_fill == 0 && *right == 0))),
                "{:?}",
                register_shapes(&mut universe, 1)
            );
            let display = register_shapes(&mut universe, 2)
                .unwrap_or_else(|| panic!("display vbox; terminal={}", terminal(&universe)));
            let [
                Shape::VBox {
                    children: display_children,
                    ..
                },
            ] = display.as_slice()
            else {
                panic!("display box: {display:?}");
            };
            assert!(
                matches!(display_children.first(), Some(Shape::HBox { children, .. }) if children.contains(&Shape::Char('A'))),
                "{display:?}"
            );
            assert!(
                display_children
                    .iter()
                    .skip(1)
                    .any(|shape| matches!(shape, Shape::HBox { .. })),
                "{display:?}"
            );
            assert!(
                matches!(register_shapes(&mut universe, 3).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }, Shape::Insert { class: 4, content: insertion, .. }]
            if line.contains(&Shape::Char('B')) && line.contains(&Shape::Char('C'))
                && insertion.as_slice() == [Shape::Kern(5 * Scaled::UNITY, KernKind::Explicit)])),
                "{:?}",
                register_shapes(&mut universe, 3)
            );
        },
    );
}

#[test]
fn paragraph_prev_graf_depth_off_save_and_replay_provenance_matrix() {
    // TeX82 §§1088--1096: one completed line publishes prev_graf and its
    // packed depth to the enclosing vlist; restricted-h vertical material
    // takes off_save instead of ending a paragraph; and a backed-up implicit
    // trigger runs once, after everypar, with its original source context.
    with_run(
        br"\font\f=cmr10 \f \hsize=100pt
          \setbox0=\vbox{\noindent g\par
            \global\count0=\prevgraf\global\dimen0=\prevdepth}",
        true,
        |_, mut state| {
            assert_eq!(count_register(&mut state, 0), 1);
            let line_depth = match register_shapes(&mut state, 0).as_deref() {
                Some([Shape::VBox { children, .. }]) => match children.as_slice() {
                    [Shape::HBox { depth, .. }] => *depth,
                    other => panic!("paragraph vlist: {other:?}"),
                },
                other => panic!("paragraph box: {other:?}"),
            };
            assert_eq!(dimen_register(&mut state, 0).raw(), line_depth);

            with_run(
                br"\hsize=100pt\maxdepth=10pt
          \noindent\vrule height0pt depth4pt width1pt\par
          \dimen1=\pagedepth",
                false,
                |_, mut page| {
                    let page_probe = detach_page_probe(&mut page);
                    assert_eq!(
                        dimen_register(&mut page, 1).raw(),
                        4 * Scaled::UNITY,
                        "outer page depth is the rule's exact 4pt depth: contents={:?} contributions={:?} depth={:?}",
                        page_probe.contents,
                        page_probe.contributions,
                        page_probe.depth
                    );
                    assert_eq!(dimen_register(&mut page, 1), page_probe.depth);

                    with_run(
                        br"\nonstopmode\setbox1=\hbox{\vskip\global\count1=21}\global\count2=22",
                        false,
                        |_, mut restricted| {
                            assert_eq!(
                                (
                                    count_register(&mut restricted, 1),
                                    count_register(&mut restricted, 2)
                                ),
                                (21, 22)
                            );
                            assert!(
                                matches!(register_shapes(&mut restricted, 1).as_deref(), Some([Shape::HBox { children, .. }]) if children.is_empty()),
                                "{:?}",
                                register_shapes(&mut restricted, 1)
                            );
                            let errors = terminal(&restricted);
                            assert_eq!(errors.matches("Missing } inserted").count(), 1, "{errors}");

                            with_run(
                                br"\nonstopmode\everypar{\global\advance\count3 by1\kern1pt}
          \setbox2=\vbox{\unhbox300\kern2pt\par}\global\count5=25",
                                false,
                                |_, mut replay| {
                                    assert_eq!(
                                        (
                                            count_register(&mut replay, 3),
                                            count_register(&mut replay, 5)
                                        ),
                                        (1, 25)
                                    );
                                    assert!(
                                        matches!(register_shapes(&mut replay, 2).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if matches!(line.as_slice(), [Shape::HBox { children: indent, .. }, Shape::Kern(first, KernKind::Explicit), Shape::Kern(second, KernKind::Explicit), ..]
                if indent.is_empty()
                    && *first == Scaled::UNITY && *second == 2 * Scaled::UNITY))),
                                        "{:?}",
                                        register_shapes(&mut replay, 2)
                                    );
                                    let errors = terminal(&replay);
                                    assert_eq!(
                                        errors.matches("Bad register code").count(),
                                        1,
                                        "{errors}"
                                    );
                                    assert!(errors.contains("\\unhbox300"), "{errors}");
                                },
                            );
                        },
                    );
                },
            );
        },
    );
}

#[test]
fn structured_material_lifecycle_delete_unbox_italic_and_recovery_matrix() {
    // TeX82 §§1097--1113: ordered node projections cover insert/vadjust/mark
    // group closure and migration, penalties, matching/nonmatching delete,
    // move/copy unbox ownership, italic correction, and forbidden-mode
    // recovery without relying on node-presence counters.
    with_run(
        br"\font\f=cmr10 \f
          \setbox0=\hbox{f\/\mark{h}\penalty7}
          \setbox1=\vbox{\insert3{\hrule height2pt}\mark{v}\penalty8}
          \setbox2=\vbox{\noindent B\vadjust{\kern4pt}\par}
          \setbox3=\hbox{\kern1pt\unkern\hskip2pt\unpenalty\unskip
                           \penalty9\unpenalty\vrule width3pt}
          \setbox4=\hbox{\kern5pt}\setbox5=\hbox{\unhcopy4\unhbox4}
          \setbox7=\vbox{\kern6pt}
          \nonstopmode\setbox6=\vbox{\/\global\count0=11
            \unskip\unkern\unpenalty\hbox{\unhbox7}\vadjust{\kern1pt}}",
        true,
        |_, mut universe| {
            let horizontal = register_shapes(&mut universe, 0);
            assert!(
                matches!(horizontal.as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Char('f'), Shape::Kern(k, KernKind::Explicit), Shape::Mark(0, mark), Shape::Penalty(7)] if *k > 0 && mark == "h")),
                "{horizontal:?}"
            );
            assert!(
                matches!(register_shapes(&mut universe, 1).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Insert { class: 3, content, .. }, Shape::Mark(0, mark), Shape::Penalty(8)]
            if matches!(content.as_slice(), [Shape::Rule(_, Some(h), _) ] if *h == 2 * Scaled::UNITY) && mark == "v"))
            );
            assert!(
                matches!(register_shapes(&mut universe, 2).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { .. }, Shape::Kern(k, KernKind::Explicit)] if *k == 4 * Scaled::UNITY))
            );
            assert!(
                matches!(register_shapes(&mut universe, 3).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Rule(Some(w), _, _)] if *w == 3 * Scaled::UNITY))
            );
            assert!(
                matches!(register_shapes(&mut universe, 5).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::Kern(5 * Scaled::UNITY, KernKind::Explicit), Shape::Kern(5 * Scaled::UNITY, KernKind::Explicit)])
            );
            assert_eq!(register_shapes(&mut universe, 4), None);
            assert!(register_shapes(&mut universe, 7).is_some());
            assert_eq!(count_register(&mut universe, 0), 11);
            let errors = terminal(&universe);
            assert!(errors.contains("You can't use `\\/' in internal vertical mode"));
            assert!(
                errors.contains("Incompatible list can't be unboxed"),
                "{errors}"
            );
            assert!(errors.contains("You can't use `\\vadjust' in internal vertical mode"));
        },
    );

    with_run(
        br"\font\f=cmr10 \f\nonstopmode
          \setbox0=\vbox{\insert0{\kern1pt}\insert254{\kern2pt}\insert255{\kern3pt}\insert256{\kern4pt}}
          \setbox2=\hbox{\kern1pt\/}\end",
        true,
        |_, mut boundaries| {
    let classes = register_shapes(&mut boundaries, 0);
    assert!(
        matches!(classes.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [
            Shape::Insert { class: 0, content: zero, .. },
            Shape::Insert { class: 254, content: high, .. },
            Shape::Insert { class: 0, content: reserved, .. },
            Shape::Insert { class: 0, content: overflow, .. },
        ]
            if zero == &[Shape::Kern(Scaled::UNITY, KernKind::Explicit)]
                && high == &[Shape::Kern(2 * Scaled::UNITY, KernKind::Explicit)]
                && reserved == &[Shape::Kern(3 * Scaled::UNITY, KernKind::Explicit)]
                && overflow == &[Shape::Kern(4 * Scaled::UNITY, KernKind::Explicit)])),
        "{classes:?}"
    );
    assert!(
        matches!(register_shapes(&mut boundaries, 2).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::Kern(Scaled::UNITY, KernKind::Explicit)])
    );
    let boundary_errors = terminal(&boundaries);
    assert!(
        boundary_errors.contains("You can't \\insert255"),
        "{boundary_errors}"
    );
            assert!(
                boundary_errors.contains("Bad register code"),
                "{boundary_errors}"
            );
        },
    );
}
