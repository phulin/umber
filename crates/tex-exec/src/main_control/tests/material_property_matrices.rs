use std::sync::Arc;

use tex_command::{FontResource, RegisteredSourceKind, SourceRegistration};
use tex_state::env::banks::{DimenParam, IntParam};
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
    Glue(i32, GlueKind, bool),
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
        stores
            .world_mut()
            .set_memory_file("cmr10.tfm", CMR10.to_vec())
            .expect("font fixture installs");
        stores
            .world_mut()
            .set_memory_file("cmr10b.tfm", CMR10.to_vec())
            .expect("second font fixture installs");
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
            } => Shape::Glue(stores.glue(*spec).width.raw(), *kind, leader.is_some()),
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
        if matches!(children.as_slice(), [Shape::Mark(0, mark), Shape::Glue(_, GlueKind::SplitTopSkip, false), Shape::Rule(None, Some(height), Some(0))] if mark == "c" && *height == 6 * Scaled::UNITY))
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
        if matches!(children.as_slice(), [Shape::Lig(first), Shape::Glue(_, _, false), Shape::Char('A'), Shape::Kern(_, KernKind::Font), Shape::Char('V'), Shape::Glue(_, _, false), Shape::Char('f'), Shape::Char('i')] if first == &['f', 'i'])),
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
        if matches!(children.as_slice(), [Shape::Char('A'), Shape::Glue(first, _, false), Shape::Char('X')] if *first == 4 * Scaled::UNITY)),
        "{:?}",
        register_shapes(&stores, 2)
    );
    assert!(
        matches!(register_shapes(&stores, 13).as_deref(), Some([Shape::HBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Char('A'), Shape::Glue(second, _, false), Shape::Char('X')] if *second == 9 * Scaled::UNITY)),
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
        if matches!(children.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Glue(w, GlueKind::Normal, false), Shape::Rule(Some(rw), Some(rh), Some(rd)), Shape::Glue(_, GlueKind::Normal, false)]
            if *k == -Scaled::UNITY && *w == 2 * Scaled::UNITY && *rw == 2 * Scaled::UNITY && *rh == 5 * Scaled::UNITY && *rd == -Scaled::UNITY))
    );
    let vertical = register_shapes(&stores, 1);
    assert!(
        matches!(vertical.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Glue(w, GlueKind::Normal, false), Shape::Rule(Some(rw), Some(rh), None)]
            if *k == -2 * Scaled::UNITY && *w == 3 * Scaled::UNITY && *rw == 4 * Scaled::UNITY && *rh == 5 * Scaled::UNITY))
            || matches!(vertical.as_deref(), Some([Shape::VBox { children, .. }])
            if matches!(children.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Glue(w, GlueKind::Normal, false), Shape::Rule(Some(rw), Some(rh), Some(0))]
                if *k == -2 * Scaled::UNITY && *w == 3 * Scaled::UNITY && *rw == 4 * Scaled::UNITY && *rh == 5 * Scaled::UNITY)),
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
        if matches!(children.as_slice(), [Shape::HBox { shift, .. }, Shape::Glue(_, _, true)] if *shift == -2 * Scaled::UNITY))
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
            if matches!(line.as_slice(), [Shape::HBox { children: indent, .. }, Shape::Kern(k, KernKind::Explicit), Shape::Char('A'), Shape::Penalty(10_000), Shape::Glue(_, GlueKind::ParFillSkip, false), Shape::Glue(_, GlueKind::RightSkip, false)] if indent.is_empty() && *k == Scaled::UNITY))),
        "{indented:?}"
    );
    let noindent = register_shapes(&stores, 1);
    assert!(
        matches!(noindent.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if matches!(line.as_slice(), [Shape::Kern(k, KernKind::Explicit), Shape::Char('B'), Shape::Penalty(10_000), Shape::Glue(_, GlueKind::ParFillSkip, false), Shape::Glue(_, GlueKind::RightSkip, false)] if *k == Scaled::UNITY))),
        "{noindent:?}"
    );
    let implicit = register_shapes(&stores, 2);
    assert!(
        matches!(implicit.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if matches!(line.as_slice(), [Shape::HBox { children: indent, .. }, Shape::Kern(k, KernKind::Explicit), Shape::Char('C'), Shape::Penalty(10_000), Shape::Glue(_, GlueKind::ParFillSkip, false), Shape::Glue(_, GlueKind::RightSkip, false)] if indent.is_empty() && *k == Scaled::UNITY))),
        "{implicit:?}"
    );
    let discardable = register_shapes(&stores, 3);
    assert!(
        matches!(discardable.as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { children: line, .. }]
            if line == &[Shape::Kern(Scaled::UNITY, KernKind::Explicit), Shape::Penalty(10_000), Shape::Glue(0, GlueKind::ParFillSkip, false), Shape::Glue(0, GlueKind::RightSkip, false)])),
        "{discardable:?}"
    );
    assert!(
        matches!(register_shapes(&stores, 4).as_deref(), Some([Shape::VBox { children, .. }])
        if matches!(children.as_slice(), [Shape::HBox { .. }, Shape::Mark(0, mark), Shape::Kern(k, KernKind::Explicit), Shape::Glue(w, _, false)] if mark == "m" && *k == 2 * Scaled::UNITY && *w == 3 * Scaled::UNITY))
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
