use std::sync::Arc;

use tex_command::{
    CommandDeliveryBoundary, CommandObservation, CommandObserver, FontResource, InputReason,
    InputTransition, ObservedToken, RecoveryKind, RegisteredSourceKind, SourceRegistration,
};
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::glue::Order;
use tex_state::node::{GlueKind, KernKind, Node, Whatsit};
use tex_state::page::PageMark;
use tex_state::scaled::Scaled;
use tex_state::token::Token;
use tex_state::{InputOpenState, Universe};

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
    Insert(u16, Vec<Shape>),
    Adjust(Vec<Shape>),
    Language(u8, u8, u8),
    MathOn(i32),
    MathOff(i32),
    Other(&'static str),
}

fn run(source: &[u8], with_font: bool) -> (MainControl, Universe) {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    control.set_fuel_limit(100_000).expect("bounded fuel");
    if with_font {
        const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
        const CMSY10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmsy10.tfm");
        const CMEX10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmex10.tfm");
        stores
            .world_mut()
            .set_memory_file("cmr10.tfm", CMR10.to_vec())
            .expect("font fixture installs");
        stores
            .world_mut()
            .set_memory_file("cmr10b.tfm", CMR10.to_vec())
            .expect("second font fixture installs");
        stores
            .world_mut()
            .set_memory_file("cmsy10.tfm", CMSY10.to_vec())
            .expect("math symbol font fixture installs");
        stores
            .world_mut()
            .set_memory_file("cmex10.tfm", CMEX10.to_vec())
            .expect("math extension font fixture installs");
        let metrics = tex_state::InputReadState::read_input_file(
            &mut stores.input_open_context(),
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
            &mut stores.input_open_context(),
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
            &mut stores.input_open_context(),
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
            &mut stores.input_open_context(),
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
    while let MainControlStep::Continue = control.step(&mut stores).expect("program executes") {}
    (control, stores)
}

#[derive(Default)]
struct Observations(Vec<CommandObservation>);

impl CommandObserver for Observations {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn run_observed(source: &[u8], with_font: bool) -> (MainControl, Universe, Observations) {
    let mut stores = Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    control.set_fuel_limit(100_000).expect("bounded fuel");
    if with_font {
        const CMR10: &[u8] = include_bytes!("../../../../tex-fonts/tests/fixtures/cm/cmr10.tfm");
        stores
            .world_mut()
            .set_memory_file("cmr10.tfm", CMR10.to_vec())
            .expect("font fixture installs");
        let metrics = tex_state::InputReadState::read_input_file(
            &mut stores.input_open_context(),
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
        .step_with_observer(&mut stores, &mut observations)
        .expect("program executes")
    {}
    (control, stores, observations)
}

fn text(stores: &Universe, tokens: tex_state::ids::TokenListId) -> String {
    stores
        .tokens(tokens)
        .iter()
        .filter_map(|token| match token {
            Token::Char { ch, .. } => Some(*ch),
            _ => None,
        })
        .collect()
}

fn macro_text(stores: &Universe, name: &str) -> String {
    let symbol = stores.symbol(name).expect("probe macro is defined");
    let meaning = stores.macro_meaning(symbol).expect("probe is a macro");
    text(stores, meaning.replacement_text())
}

fn shapes(stores: &Universe, nodes: &[Node]) -> Vec<Shape> {
    nodes
        .iter()
        .map(|node| match node {
            Node::Char { ch, .. } => Shape::Char(*ch),
            Node::Lig { orig, .. } => Shape::Lig(orig.clone()),
            Node::Kern { amount, kind } => Shape::Kern(amount.raw(), *kind),
            Node::Glue {
                spec, kind, leader, ..
            } => {
                let spec = stores.glue(*spec);
                Shape::Glue {
                    width: spec.width.raw(),
                    stretch: spec.stretch.raw(),
                    stretch_order: spec.stretch_order,
                    shrink: spec.shrink.raw(),
                    shrink_order: spec.shrink_order,
                    kind: *kind,
                    leader: leader.is_some(),
                }
            }
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
                children: shapes(stores, stores.nodes(boxed.children).testing_decoded()),
            },
            Node::VList(boxed) => Shape::VBox {
                width: boxed.width.raw(),
                height: boxed.height.raw(),
                depth: boxed.depth.raw(),
                shift: boxed.shift.raw(),
                children: shapes(stores, stores.nodes(boxed.children).testing_decoded()),
            },
            Node::Mark { class, tokens } => Shape::Mark(*class, text(stores, *tokens)),
            Node::Ins { class, content, .. } => Shape::Insert(
                *class,
                shapes(stores, stores.nodes(*content).testing_decoded()),
            ),
            Node::Adjust(adjust) => Shape::Adjust(shapes(
                stores,
                stores.nodes(adjust.content).testing_decoded(),
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

fn register_shapes(stores: &Universe, register: u16) -> Option<Vec<Shape>> {
    let root = stores.box_reg(register)?;
    Some(shapes(stores, stores.nodes(root).testing_decoded()))
}

fn boxed_children(stores: &Universe, register: u16) -> Vec<Node> {
    let root = stores.box_reg(register).expect("box register is nonvoid");
    let [Node::HList(boxed) | Node::VList(boxed)] = stores.nodes(root).testing_decoded() else {
        panic!("box register has exactly one box root")
    };
    stores.nodes(boxed.children).to_vec()
}

fn terminal(stores: &Universe) -> String {
    super::terminal_text(stores)
}

#[test]
fn vsplit_void_nonvbox_pruning_marks_and_packaging_matrix() {
    // TeX82 §§977--979: exercise both no-op exits, prefix ownership, all
    // split-mark transitions, top pruning, exact/oversized packing, and the
    // source-register replacement contract through ordered node projections.
    let (_, void) = run(br"\setbox1=\vsplit0 to5pt", false);
    assert_eq!(register_shapes(&void, 0), None);
    assert_eq!(register_shapes(&void, 1), None);

    let (_, wrong) = run(
        br"\nonstopmode\setbox0=\hbox{\kern7pt}\setbox1=\vsplit0 to5pt",
        false,
    );
    assert!(matches!(
        register_shapes(&wrong, 0).as_deref(),
        Some([Shape::HBox { children, .. }])
            if children.as_slice() == [Shape::Kern(7 * Scaled::UNITY, KernKind::Explicit)]
    ));
    assert_eq!(register_shapes(&wrong, 1), None);
    assert!(terminal(&wrong).contains("vbox"), "{}", terminal(&wrong));

    let (_, split) = run(
        br"\splittopskip=1pt
          \setbox0=\vbox{\mark{a}\hrule height4pt\mark{b}\penalty-10000
                           \kern2pt\mark{c}\hrule height6pt}
          \setbox1=\vsplit0 to4pt",
        false,
    );
    let prefix = register_shapes(&split, 1).expect("split prefix");
    let remainder = register_shapes(&split, 0).expect("split remainder");
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
            text(&split, split.page_mark(mark)),
            if mark == PageMark::SplitFirst {
                "a"
            } else {
                "b"
            }
        );
    }

    let (_, complete) = run(
        br"\setbox0=\vbox{\hrule height3pt}\setbox1=\vsplit0 to30pt",
        false,
    );
    assert_eq!(register_shapes(&complete, 0), None);
    assert!(
        matches!(register_shapes(&complete, 1).as_deref(), Some([Shape::VBox { height, children, .. }]) if *height == 30 * Scaled::UNITY && matches!(children.as_slice(), [Shape::Rule(_, Some(h), _) ] if *h == 3 * Scaled::UNITY))
    );
}

#[test]
fn vsplit_breakpoint_mark_scope_and_complete_ownership_matrix() {
    // TeX82 §§977--979: split marks are reset before every attempt, the
    // selected breakpoint is removed rather than shared, nested boxes remain
    // atomic, and ordinary save-stack restoration owns both source and target
    // registers independently of the newly packed split graph.
    let (_, cleared) = run(
        br"\setbox0=\vbox{\mark{old}\hrule height1pt\penalty-10000}
          \setbox1=\vsplit0 to1pt
          \setbox0=\vbox{\hrule height1pt}\setbox2=\vsplit0 to2pt",
        false,
    );
    assert_eq!(cleared.page_mark_value(PageMark::SplitFirst), None);
    assert_eq!(cleared.page_mark_value(PageMark::SplitBot), None);

    let (_, first) = run(
        br"\setbox0=\vbox{\penalty-10000\mark{tail}\hrule height2pt}
          \setbox1=\vsplit0 to0pt",
        false,
    );
    assert_eq!(
        register_shapes(&first, 1),
        Some(vec![Shape::VBox {
            width: 0,
            height: 0,
            depth: 0,
            shift: 0,
            children: vec![],
        }])
    );
    assert!(matches!(
        register_shapes(&first, 0).as_deref(),
        Some([Shape::VBox { children, .. }])
            if matches!(children.as_slice(), [Shape::Mark(0, mark), Shape::Glue { kind: GlueKind::SplitTopSkip, .. }, Shape::Rule(_, Some(height), _) ] if mark == "tail" && *height == 2 * Scaled::UNITY)
    ));

    let (_, middle) = run(
        br"\splittopskip=0pt
          \setbox0=\vbox{\mark{first}\hrule height2pt\mark{middle}
                           \penalty-10000\kern3pt\mark{tail}\hrule height4pt}
          \setbox1=\vsplit0 to2pt",
        false,
    );
    assert!(matches!(
        register_shapes(&middle, 1).as_deref(),
        Some([Shape::VBox { children, .. }])
            if children.as_slice() == [
                Shape::Mark(0, "first".into()),
                Shape::Rule(None, Some(2 * Scaled::UNITY), Some(0)),
                Shape::Mark(0, "middle".into()),
            ]
    ));
    assert!(matches!(
        register_shapes(&middle, 0).as_deref(),
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
    assert_eq!(
        text(&middle, middle.page_mark(PageMark::SplitFirst)),
        "first"
    );
    assert_eq!(
        text(&middle, middle.page_mark(PageMark::SplitBot)),
        "middle"
    );

    let (_, penalty_first) = run(
        br"\setbox0=\vbox{\hrule height1pt\penalty-10000\penalty-9999
                           \hrule height2pt}
          \setbox1=\vsplit0 to1pt",
        false,
    );
    assert!(matches!(
        register_shapes(&penalty_first, 1).as_deref(),
        Some([Shape::VBox { children, .. }])
            if children.as_slice() == [Shape::Rule(None, Some(Scaled::UNITY), Some(0))]
    ));
    assert!(
        matches!(
            register_shapes(&penalty_first, 0).as_deref(),
            Some([Shape::VBox { children, .. }])
                if matches!(children.as_slice(), [Shape::Glue { kind: GlueKind::SplitTopSkip, .. }, Shape::Rule(_, Some(height), _)] if *height == 2 * Scaled::UNITY)
        ),
        "{:?}",
        register_shapes(&penalty_first, 0)
    );

    let (_, nested) = run(
        br"\setbox0=\vbox{\vbox{\mark{nested}\hrule height1pt\penalty-10000
                                  \hrule height1pt}
                           \penalty-10000\mark{outer-tail}\hrule height3pt}
          \setbox1=\vsplit0 to2pt",
        false,
    );
    assert!(matches!(
        register_shapes(&nested, 1).as_deref(),
        Some([Shape::VBox { children, .. }])
            if matches!(children.as_slice(), [Shape::VBox { children: inner, .. }]
                if inner.as_slice() == [
                    Shape::Mark(0, "nested".into()),
                    Shape::Rule(None, Some(Scaled::UNITY), Some(0)),
                    Shape::Penalty(-10_000),
                    Shape::Rule(None, Some(Scaled::UNITY), Some(0)),
                ])
    ));
    assert_eq!(nested.page_mark_value(PageMark::SplitFirst), None);
    assert_eq!(nested.page_mark_value(PageMark::SplitBot), None);
    assert!(matches!(
        register_shapes(&nested, 0).as_deref(),
        Some([Shape::VBox { children, .. }])
            if matches!(children.as_slice(), [Shape::Mark(0, mark), Shape::Glue { kind: GlueKind::SplitTopSkip, .. }, Shape::Rule(_, Some(height), _)] if mark == "outer-tail" && *height == 3 * Scaled::UNITY)
    ));

    let (_, scoped) = run(
        br"\setbox0=\vbox{\hrule height8pt}\setbox1=\vbox{\kern9pt}
          {\setbox0=\vbox{\mark{local}\hrule height2pt\penalty-10000
                            \kern3pt\hrule height4pt}
           \global\setbox2=\vsplit0 to2pt
           \setbox1=\copy0}",
        false,
    );
    assert!(matches!(
        register_shapes(&scoped, 0).as_deref(),
        Some([Shape::VBox { children, .. }])
            if children.as_slice() == [Shape::Rule(None, Some(8 * Scaled::UNITY), Some(0))]
    ));
    assert!(matches!(
        register_shapes(&scoped, 1).as_deref(),
        Some([Shape::VBox { children, .. }])
            if children.as_slice() == [Shape::Kern(9 * Scaled::UNITY, KernKind::Explicit)]
    ));
    assert!(matches!(
        register_shapes(&scoped, 2).as_deref(),
        Some([Shape::VBox { children, .. }])
            if children.as_slice() == [
                Shape::Mark(0, "local".into()),
                Shape::Rule(None, Some(2 * Scaled::UNITY), Some(0)),
            ]
    ));
}

#[test]
fn text_material_character_ligkern_space_language_and_vertical_replay_matrix() {
    // TeX82 §§1032--1044: ordered projections distinguish every character
    // delivery form, ligature/kern and no-boundary handling, all space-glue
    // sources, language nodes, missing glyph recovery, and vertical replay.
    let (_, stores) = run(
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
    );
    assert_eq!(
        register_shapes(&stores, 0),
        Some(vec![Shape::HBox {
            width: register_box_width(&stores, 0),
            height: register_box_height(&stores, 0),
            depth: register_box_depth(&stores, 0),
            shift: 0,
            children: vec![Shape::Char('A'), Shape::Char('B'), Shape::Char('C')],
        }])
    );
    let ligkern = register_shapes(&stores, 1).expect("lig/kern box");
    assert!(
        matches!(ligkern.as_slice(), [Shape::HBox { children, .. }]
        if matches!(children.as_slice(), [Shape::Lig(first), Shape::Glue { leader: false, .. }, Shape::Char('A'), Shape::Kern(_, KernKind::Font), Shape::Char('V'), Shape::Glue { leader: false, .. }, Shape::Char('f'), Shape::Char('i')] if first == &['f', 'i'])),
        "{ligkern:?}"
    );
    assert_eq!(macro_text(&stores, "sfzero"), "1000");
    assert_eq!(macro_text(&stores, "sflow"), "500");
    assert_eq!(macro_text(&stores, "sfnormal"), "1000");
    assert_eq!(macro_text(&stores, "sfhigh"), "3000");
    assert!(
        matches!(register_shapes(&stores, 10).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::Char('f'), Shape::Char('i')])
    );
    assert!(
        matches!(register_shapes(&stores, 2).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Char('A'), Shape::Glue { width, leader: false, .. }, Shape::Char('X')] if *width == 4 * Scaled::UNITY)),
        "{:?}",
        register_shapes(&stores, 2)
    );
    assert!(
        matches!(register_shapes(&stores, 13).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Char('A'), Shape::Glue { width, leader: false, .. }, Shape::Char('X')] if *width == 9 * Scaled::UNITY)),
        "{:?}",
        register_shapes(&stores, 13)
    );
    assert!(
        matches!(register_shapes(&stores, 3).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::Language(7, 2, 3), Shape::Language(7, 2, 3)])
    );
    assert_eq!(stores.count(0), 1);
    assert!(
        matches!(register_shapes(&stores, 4).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }] if line.contains(&Shape::Char('A'))))
    );
    assert!(
        matches!(register_shapes(&stores, 5).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::Kern(Scaled::UNITY, KernKind::Explicit)])
    );
    assert!(terminal(&stores).contains("Missing character"));
}

#[test]
fn text_boundary_font_glue_scaling_and_cache_matrix() {
    // TeX82 §§1032--1042: boundary suppression changes the ligature program;
    // font glue and both explicit space parameters preserve all five typed
    // glue fields after space-factor scaling; equal results reuse one frozen
    // glue identity while distinct factors do not alias.
    let (_, stores) = run(
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
    );
    assert!(matches!(
        register_shapes(&stores, 0).as_deref(),
        Some([Shape::HBox { children, .. }])
            if children.as_slice() == [Shape::Lig(vec!['f', 'i'])]
    ));
    assert!(matches!(
        register_shapes(&stores, 1).as_deref(),
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
                register_shapes(&stores, register).as_deref(),
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
            register_shapes(&stores, register)
        );
    }
    for (register, width, stretch, stretch_order, shrink, shrink_order, kind) in [
        (5, 4, 1, Order::Fil, 6, Order::Fill, GlueKind::SpaceSkip),
        (6, 4, 2, Order::Fil, 3, Order::Fill, GlueKind::SpaceSkip),
        (7, 9, 6, Order::Fill, 12, Order::Fil, GlueKind::XSpaceSkip),
    ] {
        assert!(
            matches!(
                register_shapes(&stores, register).as_deref(),
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
            register_shapes(&stores, register)
        );
    }
    let cached = boxed_children(&stores, 8);
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
    assert_ne!(
        first, second,
        "each node retains independent glue ownership"
    );
    assert_eq!(
        stores.glue(*first),
        tex_state::glue::GlueSpec {
            width: Scaled::from_raw(218_453),
            stretch: Scaled::from_raw(109_226),
            stretch_order: Order::Normal,
            shrink: Scaled::from_raw(72_818),
            shrink_order: Order::Normal,
        }
    );
    assert_eq!(
        stores.glue(*second),
        tex_state::glue::GlueSpec {
            width: Scaled::from_raw(218_453),
            stretch: Scaled::from_raw(109_116),
            stretch_order: Order::Normal,
            shrink: Scaled::from_raw(72_890),
            shrink_order: Order::Normal,
        },
        "uppercase X's sfcode 999 selects a distinct cached scaling variant"
    );
    let low_box = boxed_children(&stores, 2);
    let [
        Node::Char { .. },
        Node::Glue { spec: low, .. },
        Node::Char { .. },
    ] = low_box.as_slice()
    else {
        panic!("low-factor box has one glue")
    };
    let normal_box = boxed_children(&stores, 3);
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
        register_shapes(&stores, 9).as_deref(),
        Some([Shape::HBox { children, .. }])
            if matches!(children.as_slice(), [Shape::Char('A'), Shape::Glue { width, .. }, Shape::Char('X')] if *width == 6 * Scaled::UNITY)
    ));
    assert!(matches!(
        register_shapes(&stores, 10).as_deref(),
        Some([Shape::HBox { children, .. }])
            if matches!(children.as_slice(), [Shape::Char('A'), Shape::Glue { width, .. }, Shape::Char('X')] if *width == 218_453)
    ));
}

#[test]
fn text_outer_vertical_math_illegal_meaning_and_trigger_provenance_matrix() {
    // TeX82 §§1032--1044: a character starts a paragraph in outer vertical
    // mode and becomes a math noad in math mode; `\noboundary` is illegal in
    // both modes. The horizontal case pins the exact macro expansion,
    // boundary cancellation, backed-up trigger, and resumed command order.
    let (control, modes) = run(
        br"\font\f=cmr10 \f\nonstopmode
          \everypar{\global\advance\count1 by1}
          #
          A\par
          \noboundary\par
          \setbox0=\hbox{$\noboundary$}
          $#$\par
          \xdef\noboundarymeaning{\meaning\noboundary}\count0=7",
        true,
    );
    assert_eq!(
        modes.count(1),
        3,
        "character, vertical no-boundary, and math shift each start a paragraph"
    );
    assert_eq!(control.current_mode(), crate::mode::Mode::Vertical);
    assert!(
        matches!(
            register_shapes(&modes, 0).as_deref(),
            Some([Shape::HBox { children, .. }])
                if children.as_slice() == [Shape::MathOn(0), Shape::MathOff(0)]
        ),
        "{:?}",
        register_shapes(&modes, 0)
    );
    assert_eq!(modes.count(0), 7);
    assert_eq!(macro_text(&modes, "noboundarymeaning"), r"\noboundary");
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
    let (control, stores, observations) = run_observed(source, false);
    assert_eq!(stores.count(0), 1);
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
                if command.spelling == ObservedToken::ControlSequence("noboundary".into())
                    || command.spelling == ObservedToken::ControlSequence("emit".into()) =>
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
            CommandObservation::Recovery(recovery) if recovery.kind == RecoveryKind::Backup => {
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
}

fn register_box(stores: &Universe, register: u16) -> tex_state::node::BoxNode {
    let root = stores.box_reg(register).expect("box register");
    match stores.nodes(root).testing_decoded() {
        [Node::HList(boxed) | Node::VList(boxed)] => *boxed,
        other => panic!("register {register} root: {other:?}"),
    }
}

fn register_box_width(stores: &Universe, register: u16) -> i32 {
    register_box(stores, register).width.raw()
}

fn register_box_height(stores: &Universe, register: u16) -> i32 {
    register_box(stores, register).height.raw()
}

fn register_box_depth(stores: &Universe, register: u16) -> i32 {
    register_box(stores, register).depth.raw()
}

#[test]
fn direct_material_modes_operands_page_boundary_and_group_clear_matrix() {
    // TeX82 §§1055--1062/1070: named/explicit forms, signed dimensions,
    // glue orders, rule keyword replacement, h/v/math routing, page building,
    // and normal-paragraph clearing are all independently observable.
    let (_, stores) = run(
        br"\setbox0=\hbox{\kern-1pt\hskip2pt plus3fil minus4fill
                           \vrule height1pt width2pt height5pt depth-1pt\hfil}
          \setbox1=\vbox{\kern-2pt\vskip3pt plus1fill\hrule width4pt height5pt}
          \parshape=1 1pt 9pt \hangindent=7pt \hangafter=3
          {\parshape=1 2pt 8pt \hangindent=6pt \hangafter=4}
          \par",
        false,
    );
    assert!(
        matches!(register_shapes(&stores, 0).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Glue { width, kind: GlueKind::Normal, leader: false, .. }, Shape::Rule(Some(rw), Some(rh), Some(rd)), Shape::Glue { kind: GlueKind::Normal, leader: false, .. }]
            if *k == -Scaled::UNITY && *width == 2 * Scaled::UNITY && *rw == 2 * Scaled::UNITY && *rh == 5 * Scaled::UNITY && *rd == -Scaled::UNITY))
    );
    let vertical = register_shapes(&stores, 1);
    assert!(
        matches!(vertical.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Glue { width, kind: GlueKind::Normal, leader: false, .. }, Shape::Rule(Some(rw), Some(rh), None)]
            if *k == -2 * Scaled::UNITY && *width == 3 * Scaled::UNITY && *rw == 4 * Scaled::UNITY && *rh == 5 * Scaled::UNITY))
            || matches!(vertical.as_deref(), Some([Shape::VBox { children, .. }])
            if matches!(children.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Glue { width, kind: GlueKind::Normal, leader: false, .. }, Shape::Rule(Some(rw), Some(rh), Some(0))]
                if *k == -2 * Scaled::UNITY && *width == 3 * Scaled::UNITY && *rw == 4 * Scaled::UNITY && *rh == 5 * Scaled::UNITY)),
        "{vertical:?}"
    );
    assert!(stores.paragraph_shape().is_empty());
    assert_eq!(
        stores.dimen_param(DimenParam::HANG_INDENT),
        Scaled::from_raw(0)
    );
    assert_eq!(stores.int_param(IntParam::HANG_AFTER), 1);

    let (_, page) = run(
        br"\vsize=1pt\topskip=0pt\hrule height2pt\penalty-10000\end",
        false,
    );
    assert_eq!(page.world().artifact_commits().len(), 1);

    let (_, recovery) = run(br"\setbox0=\hbox{\vrule width1pt X\kern2pt}", false);
    assert!(
        matches!(register_shapes(&recovery, 0).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Rule(Some(w), _, _), Shape::Kern(k, KernKind::Explicit)] if *w == Scaled::UNITY && *k == 2 * Scaled::UNITY))
    );
}

#[test]
fn direct_material_full_mode_named_glue_and_math_routing_matrix() {
    // TeX82 §§1055--1062: fixed glue names are exactly their explicit specs,
    // including independent stretch/shrink infinity orders. Horizontal,
    // internal-vertical, outer-vertical, and math dispatch each preserve the
    // command's typed node and perform only the mode transitions in §1090/§1095.
    let (_, named) = run(
        br"\setbox0=\hbox{\hskip0pt plus1fil}\setbox1=\hbox{\hfil}
          \setbox2=\hbox{\hskip0pt plus1fill}\setbox3=\hbox{\hfill}
          \setbox4=\hbox{\hskip0pt plus1fil minus1fil}\setbox5=\hbox{\hss}
          \setbox6=\hbox{\hskip0pt plus-1fil}\setbox7=\hbox{\hfilneg}
          \setbox8=\vbox{\vskip0pt plus1fil}\setbox9=\vbox{\vfil}
          \setbox10=\vbox{\vskip0pt plus1fill}\setbox11=\vbox{\vfill}
          \setbox12=\vbox{\vskip0pt plus1fil minus1fil}\setbox13=\vbox{\vss}
          \setbox14=\vbox{\vskip0pt plus-1fil}\setbox15=\vbox{\vfilneg}",
        false,
    );
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
            register_shapes(&named, explicit),
            register_shapes(&named, fixed),
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
                register_shapes(&named, register).as_deref(),
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
            register_shapes(&named, register)
        );
    }

    let (_, modes) = run(
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
    );
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
        register_shapes(&modes, 0).as_deref(),
        Some([Shape::HBox { children, .. }]) if children.as_slice() == direct
    ));
    assert!(matches!(
        register_shapes(&modes, 1).as_deref(),
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
            register_shapes(&modes, 2).as_deref(),
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
        register_shapes(&modes, 2),
        terminal(&modes)
    );

    let (control, outer) = run(
        br"\everypar{\global\advance\count0 by1}
          \vskip1pt\xdef\aftervskip{\the\count0}
          \hrule height1pt\xdef\afterhrule{\the\count0}
          \kern1pt\penalty0\xdef\afterkern{\the\count0}
          \hskip1pt\par\xdef\afterhskip{\the\count0}
          \vrule width1pt\par\xdef\aftervrule{\the\count0}",
        false,
    );
    assert_eq!(control.current_mode(), crate::mode::Mode::Vertical);
    assert_eq!(macro_text(&outer, "aftervskip"), "0");
    assert_eq!(macro_text(&outer, "afterhrule"), "0");
    assert_eq!(macro_text(&outer, "afterkern"), "0");
    assert_eq!(macro_text(&outer, "afterhskip"), "1");
    assert_eq!(macro_text(&outer, "aftervrule"), "2");
}

#[test]
fn direct_material_math_recovery_and_failed_keyword_token_ownership_matrix() {
    // TeX82 §§1046--1047/1055--1062: math-mode `\hrule` and `\vskip`
    // recover by inserting a math shift before either command scans an
    // operand. A failed rule keyword backs up the exact offending token;
    // nullfont recovery then proves that token executes once before the
    // following kern, rather than being swallowed by rule scanning.
    let (_, hrule) = run(br"\nonstopmode\setbox0=\hbox{$\hrule\kern2pt}", false);
    assert_eq!(
        register_shapes(&hrule, 0),
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

    let (control, vskip) = run(
        br"\nonstopmode\noindent$\vskip1pt\global\count0=7\par",
        false,
    );
    assert_eq!(vskip.count(0), 7);
    assert_eq!(control.current_mode(), crate::mode::Mode::Vertical);
    assert_eq!(terminal(&vskip).matches("Missing $ inserted").count(), 1);

    let source =
        br"\nonstopmode\tracinglostchars=1\nullfont\setbox0=\hbox{\vrule width1pt X\kern2pt}";
    let (_, stores, observations) = run_observed(source, false);
    assert_eq!(
        register_shapes(&stores, 0),
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
        terminal(&stores)
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
                if recovery.kind == RecoveryKind::Backup && recovery.tokens == [x.clone()] =>
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
                    || command.spelling == ObservedToken::ControlSequence("kern".into()) =>
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
}

#[test]
fn box_construction_targets_specs_hooks_shifts_leaders_and_register_matrix() {
    // TeX82 §§1071--1087: one ordered matrix spans constructors/specs,
    // everybox hooks, local/global targets, shifts, leaders, shipout, copy,
    // take, lastbox, vtop adjustment, and scanner recovery.
    let (_, stores) = run(
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
    );
    assert_eq!(
        stores.count(0),
        11,
        "all eleven hbox constructors run everyhbox"
    );
    assert_eq!(
        stores.count(1),
        4,
        "four vbox/vtop constructors run everyvbox"
    );
    assert_eq!(register_box_width(&stores, 1), 10 * Scaled::UNITY);
    assert_eq!(register_box_width(&stores, 2), 2 * Scaled::UNITY);
    assert_eq!(register_box_height(&stores, 4), 3 * Scaled::UNITY);
    assert!(register_shapes(&stores, 5).is_some());
    assert_eq!(register_shapes(&stores, 6), None);
    assert_eq!(register_shapes(&stores, 0), None);
    assert_eq!(register_shapes(&stores, 7), register_shapes(&stores, 8));
    assert!(
        matches!(register_shapes(&stores, 9).as_deref(), Some([Shape::HBox { children, .. }]) if children.is_empty())
    );
    assert!(
        matches!(register_shapes(&stores, 10).as_deref(), Some([Shape::HBox { children, .. }]) if matches!(children.as_slice(), [Shape::Kern(k, KernKind::Explicit)] if *k == 4 * Scaled::UNITY))
    );
    assert!(
        matches!(register_shapes(&stores, 11).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { shift, .. }, Shape::Glue { leader: true, .. }] if *shift == -2 * Scaled::UNITY))
    );
    assert!(
        matches!(register_shapes(&stores, 12).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::VBox { shift, .. }] if *shift == 3 * Scaled::UNITY))
    );
    assert_eq!(stores.world().artifact_commits().len(), 1);

    let (_, recovery) = run(
        br"\nonstopmode\setbox0=\hbox\kern2pt}\setbox1=\count0=7\setbox2=\hbox{}",
        false,
    );
    assert_eq!(recovery.count(0), 7);
    assert!(register_shapes(&recovery, 2).is_some());
    assert!(terminal(&recovery).contains("Missing { inserted"));
    assert!(terminal(&recovery).contains("Improper \\setbox"));
}

#[test]
fn paragraph_entry_endings_migration_depth_and_recovery_matrix() {
    // TeX82 §§1088--1096: explicit/implicit entry, indent ordering, empty and
    // discardable endings, vertical-trigger and group-close endings,
    // migration, internal-v versus outer-page contribution, and recovery are
    // distinguished by their exact nested node sequences.
    let (_, stores) = run(
        br"\font\f=cmr10 \f \hsize=100pt \everypar{\global\advance\count0 by1\kern1pt}
          \setbox0=\vbox{\indent A\par}
          \setbox1=\vbox{\noindent B\par}
          \setbox2=\vbox{C\par}
          \setbox3=\vbox{\noindent\hskip1pt\par}
          \setbox4=\vbox{\noindent D\mark{m}\vadjust{\kern2pt}\vskip3pt}
          \setbox5=\vbox{{\noindent E}}
          \noindent F\par\end",
        true,
    );
    assert_eq!(stores.count(0), 7);
    let indented = register_shapes(&stores, 0);
    assert!(
        matches!(indented.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if matches!(line.as_slice(), [Shape::HBox { children: indent, .. }, Shape::Kern(k, KernKind::Explicit), Shape::Char('A'), Shape::Penalty(10_000), Shape::Glue { kind: GlueKind::ParFillSkip, leader: false, .. }, Shape::Glue { kind: GlueKind::RightSkip, leader: false, .. }] if indent.is_empty() && *k == Scaled::UNITY))),
        "{indented:?}"
    );
    let noindent = register_shapes(&stores, 1);
    assert!(
        matches!(noindent.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if matches!(line.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Char('B'), Shape::Penalty(10_000), Shape::Glue { kind: GlueKind::ParFillSkip, leader: false, .. }, Shape::Glue { kind: GlueKind::RightSkip, leader: false, .. }] if *k == Scaled::UNITY))),
        "{noindent:?}"
    );
    let implicit = register_shapes(&stores, 2);
    assert!(
        matches!(implicit.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if matches!(line.as_slice(), [Shape::HBox { children: indent, .. }, Shape::Kern(k, KernKind::Explicit), Shape::Char('C'), Shape::Penalty(10_000), Shape::Glue { kind: GlueKind::ParFillSkip, leader: false, .. }, Shape::Glue { kind: GlueKind::RightSkip, leader: false, .. }] if indent.is_empty() && *k == Scaled::UNITY))),
        "{implicit:?}"
    );
    let discardable = register_shapes(&stores, 3);
    assert!(
        matches!(discardable.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if matches!(line.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Penalty(10_000), Shape::Glue { width: 0, kind: GlueKind::ParFillSkip, leader: false, .. }, Shape::Glue { width: 0, kind: GlueKind::RightSkip, leader: false, .. }] if *k == Scaled::UNITY))),
        "{discardable:?}"
    );
    assert!(
        matches!(register_shapes(&stores, 4).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { .. }, Shape::Mark(0, mark), Shape::Kern(k, KernKind::Explicit), Shape::Glue { width, leader: false, .. }] if mark == "m" && *k == 2 * Scaled::UNITY && *width == 3 * Scaled::UNITY))
    );
    assert!(
        matches!(register_shapes(&stores, 5).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }] if line.contains(&Shape::Char('E'))))
    );
    assert_eq!(stores.world().artifact_commits().len(), 1);
    assert!(!terminal(&stores).contains("Missing \\par inserted"));
    assert!(!terminal(&stores).contains("Emergency stop"));
}

#[test]
fn structured_material_lifecycle_delete_unbox_italic_and_recovery_matrix() {
    // TeX82 §§1097--1113: ordered node projections cover insert/vadjust/mark
    // group closure and migration, penalties, matching/nonmatching delete,
    // move/copy unbox ownership, italic correction, and forbidden-mode
    // recovery without relying on node-presence counters.
    let (_, stores) = run(
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
    );
    let horizontal = register_shapes(&stores, 0);
    assert!(
        matches!(horizontal.as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Char('f'), Shape::Kern(k, KernKind::Explicit), Shape::Mark(0, mark), Shape::Penalty(7)] if *k > 0 && mark == "h")),
        "{horizontal:?}"
    );
    assert!(
        matches!(register_shapes(&stores, 1).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Insert(3, content), Shape::Mark(0, mark), Shape::Penalty(8)]
            if matches!(content.as_slice(), [Shape::Rule(_, Some(h), _) ] if *h == 2 * Scaled::UNITY) && mark == "v"))
    );
    assert!(
        matches!(register_shapes(&stores, 2).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { .. }, Shape::Kern(k, KernKind::Explicit)] if *k == 4 * Scaled::UNITY))
    );
    assert!(
        matches!(register_shapes(&stores, 3).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Rule(Some(w), _, _)] if *w == 3 * Scaled::UNITY))
    );
    assert!(
        matches!(register_shapes(&stores, 5).as_deref(), Some([Shape::HBox { children, .. }])
        if children.as_slice() == [Shape::Kern(5 * Scaled::UNITY, KernKind::Explicit), Shape::Kern(5 * Scaled::UNITY, KernKind::Explicit)])
    );
    assert_eq!(register_shapes(&stores, 4), None);
    assert!(register_shapes(&stores, 7).is_some());
    assert_eq!(stores.count(0), 11);
    let errors = terminal(&stores);
    assert!(errors.contains("You can't use `\\/' in internal vertical mode"));
    assert!(
        errors.contains("Incompatible list can't be unboxed"),
        "{errors}"
    );
    assert!(errors.contains("You can't use `\\vadjust' in internal vertical mode"));

    let (_, boundaries) = run(
        br"\font\f=cmr10 \f\nonstopmode
          \setbox0=\vbox{\insert0{\kern1pt}\insert254{\kern2pt}\insert255{\kern3pt}\insert256{\kern4pt}}
          \setbox2=\hbox{\kern1pt\/}\end",
        true,
    );
    let classes = register_shapes(&boundaries, 0);
    assert!(
        matches!(classes.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Insert(0, zero), Shape::Insert(254, high), Shape::Insert(0, reserved), Shape::Insert(0, overflow)]
            if zero == &[Shape::Kern(Scaled::UNITY, KernKind::Explicit)]
                && high == &[Shape::Kern(2 * Scaled::UNITY, KernKind::Explicit)]
                && reserved == &[Shape::Kern(3 * Scaled::UNITY, KernKind::Explicit)]
                && overflow == &[Shape::Kern(4 * Scaled::UNITY, KernKind::Explicit)])),
        "{classes:?}"
    );
    assert!(
        matches!(register_shapes(&boundaries, 2).as_deref(), Some([Shape::HBox { children, .. }])
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
}
