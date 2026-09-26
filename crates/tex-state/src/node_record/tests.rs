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
fn fixed_batch_keeps_independent_keys_across_chunk_rotation_and_rollback() {
    let mut annex = AnnexHarness {
        pool: crate::fork_arena::ChunkPool::with_packed_chunk_bytes(64),
        arena: crate::fork_arena::ForkArena::new(),
    };
    let mark = annex.arena.operation_mark(&annex.pool);
    let bodies: [&[u32]; 5] = [
        &[10, 11, 12],
        &[20, 21, 22],
        &[30, 31, 32],
        &[40, 41, 42],
        &[50, 51, 52],
    ];
    let mut words: Vec<u32> = bodies
        .iter()
        .flat_map(|body| std::iter::once(0).chain(body.iter().copied()))
        .collect();
    let keys = annex
        .writer()
        .append_fixed_flat(&mut words, &[4; 5])
        .expect("batch fixed publication");
    assert_eq!(keys.len(), bodies.len());
    for (key, body) in keys.iter().zip(bodies) {
        assert_eq!(
            annex.view().resolve_fixed_array::<_, 3>(*key),
            Some(body.try_into().expect("three-word body"))
        );
    }
    assert_ne!(keys[0].words(), keys[1].words());
    assert_ne!(keys[0].words()[0..2], keys[4].words()[0..2]);
    annex
        .arena
        .restore_operation(&mut annex.pool, mark)
        .expect("rollback fixed batch");
    let replacement = annex.writer().append_fixed::<()>(&[99, 98, 97]);
    assert_eq!(
        annex.view().resolve_fixed_array::<_, 3>(replacement),
        Some([99, 98, 97])
    );
    for key in keys {
        assert!(annex.view().resolve_fixed_array::<_, 3>(key).is_none());
    }
}

#[test]
fn typed_annex_copy_preserves_contiguous_and_cross_chunk_spans() {
    let mut pool = crate::fork_arena::ChunkPool::with_packed_chunk_bytes(64);
    let mut source = crate::fork_arena::ForkArena::new();
    let mut destination = crate::fork_arena::ForkArena::new();
    for len in [8, 40] {
        let body: Vec<u32> = (0..len).map(|word| word * 17 + 3).collect();
        let source_key = NodeAnnexWriter::new(&mut pool, &mut source).append_span::<Fixed>(&body);
        let mut copier = NodeAnnexCopier::between_regions(&mut pool, &source, &mut destination);
        let copied_body = copier.detach_span(source_key).expect("source span");
        let destination_key = copier.append_span::<Fixed>(&copied_body);
        assert_eq!(copied_body.as_slice(), body);
        assert_eq!(
            NodeAnnexView::new(&pool, &destination).detach_span(destination_key),
            Some(body)
        );
    }
}

#[test]
fn box_reencoding_clears_construction_stamp_in_both_copy_paths() {
    let mut source = AnnexHarness::new();
    let stamp = crate::node_region::PageBoxSegment::from_words([1, 0, 0, 1, 1, 2, 1, 2])
        .expect("valid construction coordinates");
    for kind in [NodeKind::HList, NodeKind::VList] {
        let mut body = [0; annex::BOX_PAYLOAD_WORDS];
        body[0] = 42;
        body[28..36].copy_from_slice(&stamp.words());
        let key = source.writer().append_fixed::<annex::BoxPayload>(&body);
        let record = NodeRecord::<PageMaterialLane>::with_key(kind, 0, 0, key);
        assert_eq!(record.box_segment(source.view()), Some(stamp));

        let (same, _) = record
            .reencode_same_region(&mut source.pool, &mut source.arena, Some)
            .expect("same-region box re-encode");
        let same_body = source
            .view()
            .resolve_fixed_array::<_, { annex::BOX_PAYLOAD_WORDS }>(annex::key_from_record::<
                annex::BoxPayload,
            >(same))
            .expect("copied box payload");
        assert_eq!(&same_body[..28], &body[..28]);
        assert_eq!(&same_body[28..], &[0; annex::BOX_PAYLOAD_WORDS - 28]);
        assert_eq!(same.box_segment(source.view()), None);
        assert_eq!(record.box_segment(source.view()), Some(stamp));

        let mut destination = crate::fork_arena::ForkArena::new();
        let (between, _) = record
            .reencode_between_regions(&mut source.pool, &source.arena, &mut destination, Some)
            .expect("cross-region box re-encode");
        let destination_view = NodeAnnexView::new(&source.pool, &destination);
        let between_body = destination_view
            .resolve_fixed_array::<_, { annex::BOX_PAYLOAD_WORDS }>(annex::key_from_record::<
                annex::BoxPayload,
            >(between))
            .expect("cross-region copied box payload");
        assert_eq!(&between_body[..28], &body[..28]);
        assert_eq!(&between_body[28..], &[0; annex::BOX_PAYLOAD_WORDS - 28]);
        assert_eq!(between.box_segment(destination_view), None);
        assert_eq!(record.box_segment(source.view()), Some(stamp));
    }
}

#[test]
fn box_migration_sidecar_decodes_multiple_exclusions_and_copies_clear_key() {
    let mut source = AnnexHarness::new();
    source.writer().append_fixed::<Fixed>(&[11, 12]);
    source
        .arena
        .seal_boundary(&mut source.pool)
        .expect("seal first excluded annex chunk");
    source.writer().append_fixed::<Fixed>(&[13, 14]);
    source
        .arena
        .seal_boundary(&mut source.pool)
        .expect("seal second excluded annex chunk");
    let first = crate::node_region::PageBoxSegment::from_words([1, 0, 0, 1, 1, 2, 0, 1])
        .expect("first excluded interval");
    let second = crate::node_region::PageBoxSegment::from_words([1, 0, 0, 1, 2, 3, 1, 2])
        .expect("second excluded interval");
    let migrations = source
        .writer()
        .publish_box_migration_segments(&[first, second]);
    let sidecar_end = source.arena.payload_position_end();
    source
        .arena
        .seal_boundary(&mut source.pool)
        .expect("isolate box wrapper annex chunk");
    let segment = crate::node_region::PageBoxSegment::from_words([
        1,
        0,
        0,
        1,
        1,
        5,
        0,
        (sidecar_end + 1) as u32,
    ])
    .expect("original box construction");
    let mut body = [0; annex::BOX_PAYLOAD_WORDS];
    body[28..36].copy_from_slice(&segment.words());
    body[36..].copy_from_slice(&migrations.words());
    let key = source.writer().append_fixed::<annex::BoxPayload>(&body);
    let record = NodeRecord::<PageMaterialLane>::with_key(NodeKind::HList, 0, 0, key);
    let metadata = record
        .box_migration_metadata(source.view())
        .expect("validated original metadata");
    assert_eq!(metadata.segment, segment);
    assert_eq!(&metadata.exclusions[..2], &[first, second]);
    assert_eq!(metadata.sidecar_annex_range, Some(2..sidecar_end));
    assert_eq!(metadata.exclusions[2].node_range(), 4..4);
    assert_eq!(metadata.exclusions[2].annex_range(), 2..sidecar_end);

    let (copy, _) = record
        .reencode_same_region(&mut source.pool, &mut source.arena, Some)
        .expect("structural copy");
    assert!(copy.box_migration_metadata(source.view()).is_none());
    let mut destination = crate::fork_arena::ForkArena::new();
    let (cross_region, _) = record
        .reencode_between_regions(&mut source.pool, &source.arena, &mut destination, Some)
        .expect("cross-region structural copy");
    let destination_view = NodeAnnexView::new(&source.pool, &destination);
    assert!(
        cross_region
            .box_migration_metadata(destination_view)
            .is_none()
    );
    assert_eq!(
        record
            .box_migration_metadata(source.view())
            .expect("original sidecar remains valid")
            .exclusions
            .len(),
        3
    );
}

#[test]
fn box_migration_sidecar_rejects_bad_serial_shape_region_and_bounds() {
    let mut source = AnnexHarness::new();
    let exclusion = crate::node_region::PageBoxSegment::from_words([1, 0, 0, 1, 1, 2, 0, 1])
        .expect("excluded interval");
    let foreign = crate::node_region::PageBoxSegment::from_words([2, 0, 0, 1, 1, 2, 0, 1])
        .expect("foreign interval");
    assert!(!annex::valid_box_exclusions([1, 0, 0, 1], &[foreign]));
    assert!(!annex::valid_box_exclusions(
        [1, 0, 0, 1],
        &[exclusion, exclusion]
    ));
    source.writer().append_fixed::<Fixed>(&[11, 12]);
    source
        .arena
        .seal_boundary(&mut source.pool)
        .expect("seal excluded annex chunk");
    let valid = source.writer().publish_box_migration_segments(&[exclusion]);
    let end = source.arena.payload_position_end();
    source
        .arena
        .seal_boundary(&mut source.pool)
        .expect("isolate wrapper annex chunk");
    let segment =
        crate::node_region::PageBoxSegment::from_words([1, 0, 0, 1, 1, 3, 0, (end + 1) as u32])
            .expect("original box construction");
    let view = source.view();
    assert!(view.box_migration_segments(valid, segment).is_some());

    let mut serial = valid.words();
    serial[6] ^= 1;
    assert!(
        view.box_migration_segments(
            crate::page_node_arena::PageBoxMigrationKey::from_words(serial),
            segment
        )
        .is_none()
    );
    let bad_shape = source
        .writer()
        .append_span::<annex::BoxMigrationSegments>(&[0x424d_5347, 1]);
    let extended = crate::node_region::PageBoxSegment::from_words([
        1,
        0,
        0,
        1,
        1,
        3,
        0,
        (source.arena.payload_position_end() + 1) as u32,
    ])
    .expect("construction through malformed sidecar");
    assert!(
        source
            .view()
            .box_migration_segments(
                crate::page_node_arena::PageBoxMigrationKey::from_words(bad_shape.words()),
                extended
            )
            .is_none()
    );
}

#[test]
fn fixed_reads_validate_size_publication_and_contiguous_bounds() {
    let mut annex = AnnexHarness::new();
    let mark = annex.arena.operation_mark(&annex.pool);
    let stale = annex.writer().append_fixed::<Fixed>(&[3, 4]);
    annex
        .arena
        .restore_operation(&mut annex.pool, mark)
        .expect("restore annex");
    let live = annex.writer().append_fixed::<Fixed>(&[8, 9]);
    assert_eq!(annex.view().resolve_fixed_array::<_, 2>(live), Some([8, 9]));
    assert!(annex.view().resolve_fixed_array::<_, 2>(stale).is_none());
    assert!(annex.view().resolve_fixed_array::<_, 1>(live).is_none());
    let foreign = AnnexHarness::new();
    assert!(foreign.view().resolve_fixed_array::<_, 2>(live).is_none());

    let capacity = annex.pool.chunk_capacity();
    let _prefix = annex.writer().append_span::<()>(&vec![1; capacity - 5]);
    let spanning = annex.writer().append_span::<Fixed>(&[7, 8, 9]);
    assert_eq!(annex.view().detach_span(spanning), Some(vec![7, 8, 9]));
    assert!(annex.view().resolve_fixed_array::<_, 3>(spanning).is_none());
    let rotated = annex.writer().append_fixed::<Fixed>(&[10, 11, 12]);
    assert_eq!(
        annex.view().resolve_fixed_array::<_, 3>(rotated),
        Some([10, 11, 12])
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
            origin: crate::node::GlueSpecOrigin::Owned,
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
            record.semantic_identity(annex.view()),
            crate::node_sequence::semantic_node_identity(&node),
            "{expected_kind:?} semantic identity"
        );
        assert_eq!(
            record.decode_owned(annex.view()),
            Some(node),
            "{expected_kind:?}"
        );
    }
}

#[test]
fn glue_record_preserves_origin_independently_of_subtype_and_rejects_false_shared_zero() {
    let mut annex = AnnexHarness::new();
    for kind in [GlueKind::Normal, GlueKind::MuSkip, GlueKind::Leaders] {
        for origin in [
            crate::node::GlueSpecOrigin::SharedZero,
            crate::node::GlueSpecOrigin::Owned,
        ] {
            let node = Node::Glue {
                spec: GlueSpec::ZERO,
                kind,
                origin,
                leader: None,
            };
            let record = NodeRecord::encode_owned(node.clone(), &mut annex.writer());
            assert_eq!(record.glue_origin(), Some(origin));
            assert_eq!(record.decode_owned(annex.view()), Some(node));
        }
    }

    let nonzero = Node::Glue {
        spec: GlueSpec {
            width: Scaled::from_raw(1),
            ..GlueSpec::ZERO
        },
        kind: GlueKind::Normal,
        origin: crate::node::GlueSpecOrigin::Owned,
        leader: None,
    };
    let record = NodeRecord::encode_owned(nonzero, &mut annex.writer());
    let invalid = NodeRecord::new(
        NodeKind::Glue,
        record.subtype(),
        record.flags() | 0x20,
        record.words(),
    );
    assert!(invalid.glue_spec_kind(annex.view()).is_none());
    assert!(invalid.decode_owned(annex.view()).is_none());
}

#[test]
fn every_whatsit_subtype_round_trips() {
    let mut annex = AnnexHarness::new();
    for whatsit in whatsits() {
        let node = Node::Whatsit(whatsit);
        let record = NodeRecord::encode_owned(node.clone(), &mut annex.writer());
        assert_eq!(record.kind(), Some(NodeKind::Whatsit));
        assert_eq!(
            record.semantic_identity(annex.view()),
            crate::node_sequence::semantic_node_identity(&node),
            "whatsit semantic identity"
        );
        assert_eq!(record.decode_owned(annex.view()), Some(node));
    }
}
