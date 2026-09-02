use super::*;

enum Fixed {}

struct AnnexHarness {
    pool: crate::fork_arena::ChunkPool<u32>,
    arena: crate::fork_arena::ForkArena<u32, crate::node_region::NodeAnnexLane>,
}

impl AnnexHarness {
    fn new() -> Self {
        Self {
            pool: crate::fork_arena::ChunkPool::with_packed_chunk_bytes(65_536),
            arena: crate::fork_arena::ForkArena::new(),
        }
    }

    fn writer(&mut self) -> NodeAnnexWriter<'_> {
        NodeAnnexWriter::new(&mut self.pool, &mut self.arena)
    }

    fn view(&self) -> NodeAnnexView<'_> {
        NodeAnnexView::new(&self.pool, &self.arena)
    }
}

#[test]
fn exact_record_and_key_layouts_are_copy_only() {
    assert_eq!(core::mem::size_of::<NodeRecord>(), 32);
    assert_eq!(core::mem::align_of::<NodeRecord>(), 4);
    assert!(!core::mem::needs_drop::<NodeRecord>());
    assert_eq!(core::mem::size_of::<Option<NodeRecord>>(), 32);
    assert_eq!(core::mem::size_of::<AnnexKey<Fixed>>(), 28);
    assert_eq!(core::mem::align_of::<AnnexKey<Fixed>>(), 4);
    assert!(!core::mem::needs_drop::<AnnexKey<Fixed>>());
}

#[test]
fn paired_annex_uses_exact_u32_superblock_cells() {
    let annex = AnnexHarness::new();
    assert!(annex.pool.has_packed_payload());
    assert_eq!(annex.pool.resident_payload_slot_bytes(), 4);
    assert_eq!(annex.pool.chunk_capacity(), 16_384);
    assert_eq!((annex.pool.chunk_capacity() - 1) * 4, 65_532);
}

#[test]
fn rollback_reuse_rejects_old_publication_serial() {
    let mut annex = AnnexHarness::new();
    let _prefix = annex.writer().append_fixed::<Fixed>(&[1]);
    let mark = annex.arena.operation_mark(&annex.pool);
    let stale = annex.writer().append_fixed::<Fixed>(&[7, 8]);
    assert_eq!(annex.view().resolve_fixed_shared(stale), Some(vec![7, 8]));
    annex
        .arena
        .restore_operation(&mut annex.pool, mark)
        .expect("rollback annex publication");
    let current = annex.writer().append_fixed::<Fixed>(&[9, 10]);
    assert!(annex.view().resolve_fixed_shared(stale).is_none());
    assert_eq!(
        annex.view().resolve_fixed_shared(current),
        Some(vec![9, 10])
    );
}

#[test]
fn fixed_records_resolve_through_the_paired_annex_arena() {
    let mut annex = AnnexHarness::new();
    let fixed = annex.writer().append_fixed::<Fixed>(&[1, 2, 3]);
    assert_eq!(
        annex.view().resolve_fixed_shared(fixed),
        Some(vec![1, 2, 3])
    );
}

#[test]
fn fixed_record_rotates_instead_of_crossing_an_annex_block() {
    let mut annex = AnnexHarness::new();
    let capacity = annex.pool.chunk_capacity();
    let prefix = vec![1; capacity - 3];
    let _prefix = annex.writer().append_span::<()>(&prefix);
    let fixed = annex.writer().append_fixed::<Fixed>(&[7, 8]);
    let words = fixed.words();
    assert_eq!(
        words[0], words[3],
        "fixed record stays in one logical block"
    );
    assert_eq!(words[1], words[4], "fixed record keeps one incarnation");
    assert_eq!(annex.view().resolve_fixed_shared(fixed), Some(vec![7, 8]));
}

#[test]
fn dynamic_annex_span_crosses_exact_word_superblocks() {
    let mut annex = AnnexHarness::new();
    let _prefix = annex.writer().append_fixed::<Fixed>(&[1, 2, 3]);
    let body: Vec<_> = (0..20_000).collect();
    let key = annex.writer().append_span::<()>(&body);
    assert_eq!(annex.view().detach_span(key), Some(body));
}

#[test]
fn annex_key_rejects_a_foreign_pool_space() {
    let mut source = AnnexHarness::new();
    let key = source.writer().append_fixed::<Fixed>(&[7, 8]);
    let foreign = AnnexHarness::new();
    assert!(foreign.view().resolve_fixed_shared(key).is_none());
}

fn box_node() -> BoxNode<PageListId> {
    BoxNode::new(BoxNodeFields {
        width: Scaled::from_raw(10),
        height: Scaled::from_raw(20),
        depth: Scaled::from_raw(3),
        shift: Scaled::from_raw(-4),
        box_lr: BoxLr::Reversed,
        glue_set: GlueSetRatio::from_ratio_parts(-3, 7),
        glue_sign: Sign::Shrinking,
        glue_order: Order::Fill,
        children: PageListId::empty(),
    })
}

fn token_key(seed: u32) -> NodeTokenKey {
    NodeTokenKey::from_coordinates([seed, 2, 3, 4, 5, 6])
}

fn whatsits() -> Vec<Whatsit> {
    vec![
        Whatsit::OpenOut {
            slot: StreamSlot::new(3),
            path: "out-µ.txt".into(),
        },
        Whatsit::CloseOut {
            slot: Some(StreamSlot::new(4)),
        },
        Whatsit::CloseOut { slot: None },
        Whatsit::DeferredWrite {
            sink: PrintSink::Stream(StreamSlot::new(5)),
            tokens: token_key(10),
        },
        Whatsit::Special {
            class: "pdf:code".into(),
            payload: vec![0, 1, 2, 255, 7],
        },
        Whatsit::DeferredSpecial {
            class: "pdf:code".into(),
            tokens: token_key(11),
        },
        Whatsit::PdfReferenceObject { object: 17 },
        Whatsit::PdfAccessibility(PdfAccessibilityControl::InterwordSpaceOff),
        Whatsit::PdfAnnotation { object: 18 },
        Whatsit::PdfLinkStart { object: 19 },
        Whatsit::PdfLinkEnd { object: 19 },
        Whatsit::PdfRunningLink(true),
        Whatsit::PdfLiteral {
            mode: PdfLiteralMode::Direct,
            payload: b"q 1 0 0 1".to_vec(),
        },
        Whatsit::DeferredPdfLiteral {
            mode: PdfLiteralMode::Page,
            tokens: token_key(12),
        },
        Whatsit::PdfSetMatrix {
            payload: b"1 0 0 1".to_vec(),
        },
        Whatsit::PdfSave,
        Whatsit::PdfRestore,
        Whatsit::PdfColorStack {
            id: 2,
            action: crate::PdfColorStackAction::Set(vec![1, 2, 3]),
        },
        Whatsit::PdfColorStack {
            id: 2,
            action: crate::PdfColorStackAction::Push(vec![4, 5]),
        },
        Whatsit::PdfColorStack {
            id: 2,
            action: crate::PdfColorStackAction::Pop,
        },
        Whatsit::PdfColorStack {
            id: 2,
            action: crate::PdfColorStackAction::Current,
        },
        Whatsit::PdfSavePos,
        Whatsit::PdfSnapRefPoint,
        Whatsit::PdfSnapY {
            glue: GlueSpec {
                width: Scaled::from_raw(1),
                stretch: Scaled::from_raw(2),
                stretch_order: Order::Fil,
                shrink: Scaled::from_raw(3),
                shrink_order: Order::Fill,
            },
        },
        Whatsit::PdfSnapYComp { ratio: 511 },
        Whatsit::PdfRefXForm {
            object: 20,
            width: Scaled::from_raw(1),
            height: Scaled::from_raw(2),
            depth: Scaled::from_raw(3),
        },
        Whatsit::PdfRefXImage {
            object: 21,
            width: Scaled::from_raw(4),
            height: Scaled::from_raw(5),
            depth: Scaled::from_raw(6),
        },
        Whatsit::PdfDestination(Box::new(PdfDestinationNode {
            identifier: NodePdfActionIdentifier::Name(token_key(13)),
            structure: Some(22),
            kind: PdfDestinationKind::FitRectangle(crate::PdfAnnotationDimensions {
                width: Some(Scaled::from_raw(7)),
                height: None,
                depth: Some(Scaled::from_raw(9)),
            }),
        })),
        Whatsit::PdfThread(Box::new(PdfThreadNode {
            identifier: NodePdfActionIdentifier::Number(23),
            dimensions: crate::PdfAnnotationDimensions {
                width: None,
                height: Some(Scaled::from_raw(10)),
                depth: None,
            },
            attributes: token_key(14),
            running: true,
        })),
        Whatsit::PdfEndThread,
        Whatsit::Language {
            language: 7,
            left_hyphen_min: 2,
            right_hyphen_min: 3,
        },
    ]
}

fn all_node_kinds() -> Vec<Node> {
    let empty = PageListId::empty();
    let glue = GlueSpec {
        width: Scaled::from_raw(10),
        stretch: Scaled::from_raw(2),
        stretch_order: Order::Fill,
        shrink: Scaled::from_raw(1),
        shrink_order: Order::Fil,
    };
    vec![
        Node::Char {
            font: crate::font::NULL_FONT,
            ch: 'λ',
            origin: OriginId::from_raw(88),
        },
        Node::Lig {
            font: crate::font::NULL_FONT,
            ch: 'ﬃ',
            orig: vec!['f', 'f', 'i'],
            left_hit: true,
            right_hit: false,
            origins: vec![
                OriginId::from_raw(1),
                OriginId::from_raw(2),
                OriginId::from_raw(3),
            ],
        },
        Node::Kern {
            amount: Scaled::from_raw(-11),
            kind: KernKind::Auto,
        },
        Node::MarginKern {
            amount: Scaled::from_raw(12),
            side: MarginKernSide::Right,
            font: crate::font::NULL_FONT,
            ch: b'A',
        },
        Node::Glue {
            spec: glue,
            kind: GlueKind::Cleaders,
            leader: Some(LeaderPayload::HList(box_node())),
        },
        Node::Penalty(-50),
        Node::Rule {
            width: Some(Scaled::from_raw(1)),
            height: None,
            depth: Some(Scaled::from_raw(3)),
        },
        Node::HList(box_node()),
        Node::VList(box_node()),
        Node::Unset(UnsetNode::new(UnsetNodeFields {
            kind: UnsetKind::VBox,
            width: Scaled::from_raw(1),
            height: Scaled::from_raw(2),
            depth: Scaled::from_raw(3),
            span_count: 65_535,
            stretch: Scaled::from_raw(4),
            stretch_order: Order::Filll,
            shrink: Scaled::from_raw(5),
            shrink_order: Order::Fil,
            children: empty,
        })),
        Node::Disc {
            kind: DiscKind::AutomaticHyphen,
            pre: empty,
            post: empty,
            replace: empty,
            physical_replace_count: 255,
        },
        Node::Mark {
            class: 65_535,
            tokens: token_key(20),
        },
        Node::Ins {
            class: 65_535,
            size: Scaled::from_raw(6),
            split_top_skip: glue,
            split_max_depth: Scaled::from_raw(7),
            floating_penalty: -100,
            content: empty,
        },
        Node::Whatsit(whatsits().remove(0)),
        Node::MathOn(Scaled::from_raw(8)),
        Node::MathOff(Scaled::from_raw(9)),
        Node::Direction(crate::node::Direction::BeginR),
        Node::MathNoad(MathNoad {
            kind: NoadKind::Accent {
                accent: MathChar {
                    family: 15,
                    character: '^',
                    origin: OriginId::from_raw(91),
                },
            },
            nucleus: MathField::MathChar(MathChar {
                family: 3,
                character: 'x',
                origin: OriginId::from_raw(92),
            }),
            subscript: MathField::SubBox(empty),
            superscript: MathField::SubMlist(empty),
        }),
        Node::FractionNoad(MathFraction {
            numerator: empty,
            denominator: empty,
            thickness: FractionThickness::Explicit(Scaled::from_raw(-1)),
            left_delimiter: Some(0),
            right_delimiter: Some(u32::MAX),
        }),
        Node::MathStyle(MathStyle::ScriptScript),
        Node::MathChoice(MathChoice {
            display: empty,
            text: empty,
            script: empty,
            script_script: empty,
        }),
        Node::MathList(MathListNode {
            display: true,
            content: empty,
        }),
        Node::Nonscript,
        Node::Adjust(AdjustNode {
            content: empty,
            pre: true,
        }),
    ]
}

#[test]
fn every_node_kind_round_trips_through_record_and_annex() {
    let mut annex = AnnexHarness::new();
    let nodes = all_node_kinds();
    assert_eq!(nodes.len(), NodeKind::ALL.len());
    for (node, expected_kind) in nodes.into_iter().zip(NodeKind::ALL) {
        assert_eq!(node.kind(), expected_kind);
        let record = NodeRecord::encode_owned(node.clone(), &mut annex.writer());
        assert_eq!(record.kind(), Some(expected_kind));
        assert_eq!(
            record.decode_owned(annex.view()),
            Some(node),
            "{expected_kind:?}"
        );
    }
}

#[test]
fn every_whatsit_subtype_round_trips() {
    let mut annex = AnnexHarness::new();
    for whatsit in whatsits() {
        let node = Node::Whatsit(whatsit);
        let record = NodeRecord::encode_owned(node.clone(), &mut annex.writer());
        assert_eq!(record.kind(), Some(NodeKind::Whatsit));
        assert_eq!(record.decode_owned(annex.view()), Some(node));
    }
}
