//! Synthetic shapes and a measured harness for the explicit, opt-in
//! node-copy timing tier.

use super::super::*;

/// Synthetic shapes for the explicit, opt-in node-copy timing tier.
#[derive(Clone, Copy, Debug)]
pub enum ExplicitCopyShape {
    Inline,
    FixedAnnex,
    Nested,
    VariableSpan,
    /// Typeset lines: hboxes of characters, interword glue, font kerns,
    /// ligatures, and hyphen discretionaries, separated by baseline glue.
    Paragraph,
    /// Code-listing lines: every character in its own fixed-width hbox,
    /// bracketed by color-stack whatsits, as `listings` produces.
    Listing,
}

/// Source and destination owners for the opt-in explicit-copy timing tier.
/// Construction and rollback are separate from the measured copy method.
pub struct ExplicitCopyHarness {
    pool: NodePool,
    source: NodeRegion<PageRole>,
    root: RegionRoot<PageRole>,
    destination: NodeRegion<DurableRole>,
    node_mark: crate::fork_arena::OperationMark<PageMaterialLane>,
    annex_mark: crate::fork_arena::OperationMark<NodeAnnexLane>,
    copied_nodes: usize,
}

impl ExplicitCopyHarness {
    pub fn new(shape: ExplicitCopyShape, nodes: usize) -> Self {
        use crate::glue::Order;
        use crate::node::{BoxLr, BoxNode, BoxNodeFields, Sign, Whatsit};
        use crate::scaled::{GlueSetRatio, Scaled};

        assert!(nodes > 0);
        let mut pool = NodePool::new();
        let mut source = pool.start_region::<PageRole>().expect("source region");
        let child = if matches!(shape, ExplicitCopyShape::Nested) {
            source
                .publish_owned(&mut pool, [Node::Penalty(7)])
                .expect("shared source child")
                .list
        } else {
            PageListId::empty()
        };
        let make_box = || {
            Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(0),
                box_lr: BoxLr::Normal,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: Order::Normal,
                children: child,
            }))
        };
        let mut copied_nodes = nodes
            * if matches!(shape, ExplicitCopyShape::Nested) {
                2
            } else {
                1
            };
        if matches!(
            shape,
            ExplicitCopyShape::Paragraph | ExplicitCopyShape::Listing
        ) {
            let (root, copied) = Self::paragraph(
                &mut pool,
                &mut source,
                nodes,
                matches!(shape, ExplicitCopyShape::Listing),
            );
            copied_nodes = copied;
            let destination = pool
                .start_region::<DurableRole>()
                .expect("destination region");
            let node_mark = destination.pub_arena.operation_mark(&pool.chunks);
            let annex_mark = destination.annex_arena.operation_mark(&pool.annex_chunks);
            return Self {
                pool,
                source,
                root,
                destination,
                node_mark,
                annex_mark,
                copied_nodes,
            };
        }
        let root = source
            .publish_owned(
                &mut pool,
                (0..nodes).map(|index| match shape {
                    ExplicitCopyShape::Inline => Node::Penalty(index as i32),
                    ExplicitCopyShape::FixedAnnex | ExplicitCopyShape::Nested => make_box(),
                    ExplicitCopyShape::VariableSpan => Node::Whatsit(Whatsit::Special {
                        class: "copy-profile".into(),
                        payload: vec![index as u8; 128],
                    }),
                    ExplicitCopyShape::Paragraph | ExplicitCopyShape::Listing => {
                        unreachable!()
                    }
                }),
            )
            .expect("source root");
        let destination = pool
            .start_region::<DurableRole>()
            .expect("destination region");
        let node_mark = destination.pub_arena.operation_mark(&pool.chunks);
        let annex_mark = destination.annex_arena.operation_mark(&pool.annex_chunks);
        Self {
            pool,
            source,
            root,
            destination,
            node_mark,
            annex_mark,
            copied_nodes,
        }
    }

    /// Publishes `lines` typeset lines and returns the root with the total
    /// recursive node count.
    fn paragraph(
        pool: &mut NodePool,
        source: &mut NodeRegion<PageRole>,
        lines: usize,
        listing: bool,
    ) -> (RegionRoot<PageRole>, usize) {
        use crate::glue::{GlueSpec, Order};
        use crate::node::{
            BoxLr, BoxNode, BoxNodeFields, DiscKind, GlueKind, GlueSpecOrigin, KernKind, Sign,
        };
        use crate::scaled::{GlueSetRatio, Scaled};
        use crate::token::OriginId;

        let font = crate::font::NULL_FONT;
        let ch = |ch| Node::Char {
            font,
            ch,
            origin: OriginId::default(),
        };
        let glue = |kind| Node::Glue {
            spec: GlueSpec {
                width: Scaled::from_raw(218_453),
                stretch: Scaled::from_raw(109_226),
                stretch_order: Order::Normal,
                shrink: Scaled::from_raw(72_818),
                shrink_order: Order::Normal,
            },
            kind,
            origin: GlueSpecOrigin::Owned,
            leader: None,
        };
        let hbox = |width, children| {
            Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(width),
                height: Scaled::from_raw(450_000),
                depth: Scaled::from_raw(120_000),
                shift: Scaled::from_raw(0),
                box_lr: BoxLr::Normal,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: Order::Normal,
                children,
            }))
        };
        let color = |action| Node::Whatsit(crate::node::Whatsit::PdfColorStack { id: 0, action });
        let mut copied = 0;
        let mut line_nodes = Vec::new();
        for line in 0..lines {
            let mut content = Vec::new();
            if listing {
                for column in 0..40 {
                    if column % 8 == 0 {
                        content.push(color(crate::PdfColorStackAction::Push(
                            b"0 0 1 rg 0 0 1 RG".to_vec(),
                        )));
                    }
                    let glyph = source
                        .publish_owned(pool, [ch(char::from(b'a' + (column % 26) as u8))])
                        .expect("listing glyph")
                        .list;
                    copied += 1;
                    content.push(hbox(340_000, glyph));
                    if column % 8 == 7 {
                        content.push(color(crate::PdfColorStackAction::Pop));
                        content.push(glue(GlueKind::Normal));
                    }
                }
            }
            let words = if listing { 0 } else { 10 };
            for word in 0..words {
                if word != 0 {
                    content.push(glue(GlueKind::SpaceSkip));
                }
                for letter in 0..5 {
                    content.push(ch(char::from(b'a' + ((line + word + letter) % 26) as u8)));
                }
                match word % 5 {
                    1 => content.push(Node::Kern {
                        amount: Scaled::from_raw(-18_000),
                        kind: KernKind::Font,
                    }),
                    2 => content.push(Node::Lig {
                        font,
                        ch: 'ﬁ',
                        orig: vec!['f', 'i'],
                        left_hit: false,
                        right_hit: false,
                        origins: vec![],
                    }),
                    3 => {
                        let pre = source
                            .publish_owned(pool, [ch('-')])
                            .expect("hyphen pre-break")
                            .list;
                        copied += 1;
                        content.push(Node::Disc {
                            kind: DiscKind::AutomaticHyphen,
                            pre,
                            post: PageListId::empty(),
                            replace: PageListId::empty(),
                            physical_replace_count: 0,
                        });
                    }
                    _ => {}
                }
            }
            copied += content.len();
            let children = source
                .publish_owned(pool, content)
                .expect("line content")
                .list;
            if line != 0 {
                line_nodes.push(glue(GlueKind::BaselineSkip));
            }
            line_nodes.push(hbox(22_000_000, children));
        }
        copied += line_nodes.len();
        let root = source
            .publish_owned(pool, line_nodes)
            .expect("paragraph root");
        (root, copied)
    }

    /// Runs one exact copy; the caller must subsequently restore this harness.
    pub fn copy_once(&mut self) -> usize {
        let before = self.destination.pub_arena.counters().source_nodes_copied;
        let copied = copy_region_root_into(
            &mut self.pool,
            &self.source,
            self.root,
            &mut self.destination,
            false,
        )
        .expect("profile copy");
        assert_eq!(copied.list.len(), self.root.list.len());
        assert_eq!(
            self.destination.pub_arena.counters().source_nodes_copied - before,
            self.copied_nodes as u64
        );
        self.copied_nodes
    }

    pub fn restore(&mut self) {
        self.destination
            .pub_arena
            .restore_operation(&mut self.pool.chunks, self.node_mark)
            .expect("restore measured node suffix");
        self.destination
            .annex_arena
            .restore_operation(&mut self.pool.annex_chunks, self.annex_mark)
            .expect("restore measured annex suffix");
    }
}
