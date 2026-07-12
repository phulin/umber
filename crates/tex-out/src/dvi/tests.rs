use super::opcodes::{
    BOP, DEN, DOWN1, EOP, FNT_DEF1, FNT_NUM_0, FNT1, ID_BYTE, NUM, POST, POST_POST, PRE, PUSH,
    PUT_RULE, RIGHT1, SET_RULE, SET1, XXX1, XXX4,
};
use super::{DviError, DviStreamWriter, write_dvi};
use crate::{
    BoxNode, ContentHash, FontResource, GlueKind, GlueOrder, GlueSetRatio, GlueSign, GlueSpec,
    JobInfo, LeaderPayload, PageArtifact, PageEffect, PageNode,
};
use tex_arith::Scaled;

const W0: u8 = 147;
const W1: u8 = 148;
const W3: u8 = 150;
const X0: u8 = 152;
const X3: u8 = 155;
const Y0: u8 = 161;
const Y1: u8 = 162;

#[derive(Default)]
struct ChunkSink {
    chunks: Vec<Vec<u8>>,
}

impl std::io::Write for ChunkSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.chunks.push(bytes.to_vec());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn streaming_writer_flushes_preamble_each_page_and_postamble() {
    let pages = [glyph_page(1), glyph_page(2)];
    let expected = write_dvi(&pages).expect("slice compatibility writer");
    let mut writer = DviStreamWriter::new(ChunkSink::default());
    writer.write_page(&pages[0]).expect("write first page");
    writer.write_page(&pages[1]).expect("write second page");
    let sink = writer.finish().expect("finish stream");

    assert_eq!(sink.chunks.len(), 4);
    assert_eq!(sink.chunks.concat(), expected);
    assert_eq!(sink.chunks[0][0], PRE);
    assert_eq!(sink.chunks[1][0], BOP);
    assert_eq!(sink.chunks[2][0], BOP);
    assert_eq!(sink.chunks[3][0], POST);
}

#[test]
fn streaming_writer_rejects_cross_page_font_identity_conflicts() {
    let first = glyph_page(1);
    let mut second = glyph_page(2);
    second.testing_mut().fonts[0].name = "incompatible".to_owned();
    let mut writer = DviStreamWriter::new(Vec::new());

    writer.write_page(&first).expect("write first page");
    assert_eq!(
        writer.write_page(&second),
        Err(DviError::InconsistentFontResource { font_id: 3 })
    );
    assert!(matches!(writer.finish(), Err(DviError::Poisoned)));
}

#[test]
fn writes_preamble_bop_body_and_postamble() {
    let dvi = write_dvi(&[glyph_page(7)]).expect("DVI writes");
    let bop = 16;
    let body = page_body(&dvi, bop);
    let mut expected_body = vec![DOWN1, 100];
    expected_body.extend(font_def_bytes(3, "cmr10"));
    expected_body.extend([FNT_NUM_0 + 3, b'A']);

    assert_eq!(dvi[0], PRE);
    assert_eq!(dvi[1], ID_BYTE);
    assert_eq!(be_i32(&dvi, 2), NUM);
    assert_eq!(be_i32(&dvi, 6), DEN);
    assert_eq!(be_i32(&dvi, 10), 1200);
    assert_eq!(dvi[14], 1);
    assert_eq!(dvi[15], b'B');

    assert_eq!(dvi[bop], BOP);
    assert_eq!(be_i32(&dvi, bop + 1), 7);
    assert_eq!(be_i32(&dvi, bop + 41), -1);
    assert_eq!(body, expected_body);

    let eop = page_eop(&dvi, bop);
    let post = eop + 1;
    assert_eq!(dvi[eop], EOP);
    assert_eq!(dvi[post], POST);
    assert_eq!(be_i32(&dvi, post + 1), bop as i32);
    assert_eq!(be_i32(&dvi, post + 5), NUM);
    assert_eq!(be_i32(&dvi, post + 9), DEN);
    assert_eq!(be_i32(&dvi, post + 13), 1200);
    assert_eq!(be_i32(&dvi, post + 17), 130);
    assert_eq!(be_i32(&dvi, post + 21), 300);
    assert_eq!(be_u16(&dvi, post + 25), 0);
    assert_eq!(be_u16(&dvi, post + 27), 1);

    assert_font_def(&dvi, post + 29, 3, "cmr10");
    let post_post = post + 50;
    assert_eq!(dvi[post_post], POST_POST);
    assert_eq!(be_i32(&dvi, post_post + 1), post as i32);
    assert_eq!(dvi[post_post + 5], ID_BYTE);
    assert!(dvi[post_post + 6..].iter().all(|&byte| byte == 223));
    assert!(dvi[post_post + 6..].len() >= 4);
    assert_eq!(dvi.len() % 4, 0);
}

#[test]
fn page_offsets_initialize_tex82_shipout_coordinates() {
    let mut page = empty_page(0);
    page.job.h_offset = sp(7);
    page.job.v_offset = sp(9);
    page.fonts.push(font_resource(0, "cmr10"));
    page.root = vlist(
        1,
        10,
        0,
        vec![hlist(1, 10, 0, vec![char_node(0, b'A' as u32, 1)])],
    );

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);

    assert_eq!(&body[..5], &[DOWN1, 19, PUSH, RIGHT1, 7]);
    let post = page_eop(&dvi, 16) + 1;
    assert_eq!(be_i32(&dvi, post + 17), 19);
    assert_eq!(be_i32(&dvi, post + 21), 8);
}

#[test]
fn chains_bop_pointers_across_pages() {
    let dvi = write_dvi(&[empty_page(1), empty_page(2)]).expect("DVI writes");
    let first_bop = 16;
    let second_bop = 62;
    let post = 108;

    assert_eq!(dvi[first_bop], BOP);
    assert_eq!(be_i32(&dvi, first_bop + 1), 1);
    assert_eq!(be_i32(&dvi, first_bop + 41), -1);
    assert_eq!(dvi[page_eop(&dvi, first_bop)], EOP);
    assert_eq!(dvi[second_bop], BOP);
    assert_eq!(be_i32(&dvi, second_bop + 1), 2);
    assert_eq!(be_i32(&dvi, second_bop + 41), first_bop as i32);
    assert_eq!(dvi[post], POST);
    assert_eq!(be_i32(&dvi, post + 1), second_bop as i32);
    assert_eq!(be_u16(&dvi, post + 27), 2);
}

#[test]
fn permits_page_local_offset_changes() {
    let first = empty_page(1);
    let mut second = empty_page(2);
    second.job.h_offset = sp(17);
    second.job.v_offset = sp(-23);

    write_dvi(&[first, second]).expect("page offsets are not DVI preamble identity");
}

#[test]
fn defines_fonts_at_first_use_and_uses_fnt_num_or_fnt1() {
    let mut page = empty_page(0);
    page.fonts = (0..65)
        .map(|id| font_resource(id, &format!("f{id:02}")))
        .collect();
    page.root = hlist(
        65,
        5,
        0,
        (0..65).map(|id| char_node(id, b'A' as u32, 1)).collect(),
    );

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);

    let first_font = find_font_def(body, b"f00", 0).expect("body f00 def");
    assert_eq!(body[first_font], FNT_DEF1);
    assert_eq!(body[first_font + 1], 0);
    assert_eq!(body[first_font + 19], FNT_NUM_0);

    let font64 = find_font_def(body, b"f64", 0).expect("body f64 def");
    assert_eq!(body[font64], FNT_DEF1);
    assert_eq!(body[font64 + 1], 64);
    assert_eq!(&body[font64 + 19..font64 + 21], &[FNT1, 64]);

    let post = page_eop(&dvi, 16) + 1;
    let post_f00 = find_font_def(&dvi, b"f00", post).expect("post f00 def");
    let post_f64 = find_font_def(&dvi, b"f64", post).expect("post f64 def");
    assert!(post_f64 < post_f00);
}

#[test]
fn set1_is_used_for_high_tex82_character_codes() {
    let mut page = glyph_page(0);
    page.root = hlist(1, 3, 0, vec![char_node(3, 200, 1)]);

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);

    assert!(body.windows(2).any(|window| window == [SET1, 200]));
}

#[test]
fn vertical_movement_reuses_y_registers() {
    let mut page = empty_page(0);
    page.fonts.push(font_resource(0, "cmr10"));
    page.root = vlist(
        10,
        0,
        0,
        vec![
            hlist(1, 10, 0, vec![char_node(0, b'A' as u32, 1)]),
            PageNode::Kern {
                amount: sp(10),
                kind: crate::KernKind::Explicit,
            },
            hlist(1, 0, 0, vec![char_node(0, b'B' as u32, 1)]),
        ],
    );

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);

    assert_eq!(&body[0..2], &[Y1, 10]);
    assert!(body.contains(&Y0));
}

#[test]
fn horizontal_movement_reuses_w_registers() {
    let mut page = empty_page(0);
    page.root = hlist(
        23,
        1,
        0,
        vec![
            rule_node(1, 1, 0),
            kern_node(10),
            rule_node(1, 1, 0),
            kern_node(10),
            rule_node(1, 1, 0),
        ],
    );

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);

    assert!(body.windows(2).any(|window| window == [W1, 10]));
    assert!(body.contains(&W0));
}

#[test]
fn positive_hlist_shift_moves_nested_box_up() {
    let mut page = empty_page(0);
    page.fonts.push(font_resource(0, "cmr10"));
    let mut raised = box_node(1, 7, 0, vec![char_node(0, b'B' as u32, 1)]);
    raised.shift = sp(5);
    page.root = hlist(
        2,
        10,
        0,
        vec![char_node(0, b'A' as u32, 1), PageNode::HList(raised)],
    );

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);

    assert!(
        body.windows(2).any(|window| window == [DOWN1, 251]),
        "positive hlist shift should emit an upward DVI movement"
    );
}

#[test]
fn rules_with_negative_width_still_move_without_rule_output() {
    let mut page = empty_page(0);
    page.root = hlist(-4, 1, 0, vec![rule_node(-5, 1, 0), rule_node(1, 1, 0)]);

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);
    let first_rule = body
        .iter()
        .position(|&byte| byte == SET_RULE)
        .expect("visible rule");

    assert_eq!(&body[first_rule - 4..first_rule], &[RIGHT1, 251, DOWN1, 1]);
}

#[test]
fn vlist_rules_use_put_rule_and_running_width() {
    let mut page = empty_page(0);
    page.root = vlist(
        4,
        9,
        0,
        vec![PageNode::Rule {
            width: None,
            height: Some(sp(7)),
            depth: Some(sp(2)),
        }],
    );

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);
    let put_rule = body
        .iter()
        .position(|&byte| byte == PUT_RULE)
        .expect("put_rule");

    assert_eq!(be_i32(body, put_rule + 1), 9);
    assert_eq!(be_i32(body, put_rule + 5), 4);
}

#[test]
fn glue_set_is_rounded_cumulatively() {
    let mut page = empty_page(0);
    let glue = GlueSpec {
        width: sp(10),
        stretch: sp(5),
        stretch_order: GlueOrder::Normal,
        shrink: sp(0),
        shrink_order: GlueOrder::Normal,
    };
    page.root = PageNode::HList(BoxNode {
        width: sp(26),
        height: sp(1),
        depth: sp(0),
        shift: sp(0),
        glue_set: GlueSetRatio::from_raw(500_000),
        glue_sign: GlueSign::Stretching,
        glue_order: GlueOrder::Normal,
        children: vec![
            rule_node(1, 1, 0),
            PageNode::Glue {
                spec: glue,
                kind: GlueKind::Normal,
                leader: None,
            },
            rule_node(1, 1, 0),
            PageNode::Glue {
                spec: glue,
                kind: GlueKind::Normal,
                leader: None,
            },
            rule_node(1, 1, 0),
        ],
    });

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);

    assert!(body.windows(2).any(|window| window == [RIGHT1, 13]));
    assert!(body.windows(2).any(|window| window == [RIGHT1, 12]));
}

#[test]
fn cumulative_glue_rounding_matches_tex82_w0_x0_sequence() {
    let mut page = empty_page(0);
    let glue = GlueSpec {
        width: sp(218_453),
        stretch: sp(109_226),
        stretch_order: GlueOrder::Normal,
        shrink: sp(72_818),
        shrink_order: GlueOrder::Normal,
    };
    let mut children = vec![rule_node(1, 1, 0)];
    for _ in 0..7 {
        children.push(PageNode::Glue {
            spec: glue,
            kind: GlueKind::Normal,
            leader: None,
        });
        children.push(rule_node(1, 1, 0));
    }
    page.root = PageNode::HList(BoxNode {
        width: sp(1_592_438),
        height: sp(1),
        depth: sp(0),
        shift: sp(0),
        glue_set: GlueSetRatio::from_ratio_parts(2_781, 33_608),
        glue_sign: GlueSign::Stretching,
        glue_order: GlueOrder::Normal,
        children,
    });

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);
    let rules: Vec<_> = body
        .iter()
        .enumerate()
        .filter_map(|(index, &opcode)| (opcode == SET_RULE).then_some(index))
        .collect();

    assert_eq!(rules.len(), 8);
    assert_eq!(body[rules[1] - 4], W3);
    assert_eq!(body[rules[2] - 4], X3);
    assert!(rules[3..=6].iter().all(|&rule| body[rule - 1] == W0));
    assert_eq!(body[rules[7] - 1], X0);
}

#[test]
fn hlist_rule_leaders_use_glue_width_and_running_height_depth() {
    let mut page = empty_page(0);
    page.root = hlist(
        70_000,
        40_000,
        20_000,
        vec![PageNode::Glue {
            spec: GlueSpec {
                width: sp(70_000),
                stretch: sp(0),
                stretch_order: GlueOrder::Normal,
                shrink: sp(0),
                shrink_order: GlueOrder::Normal,
            },
            kind: GlueKind::Leaders,
            leader: Some(LeaderPayload::Rule {
                width: Some(sp(1)),
                height: None,
                depth: None,
            }),
        }],
    );

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);
    let set_rule = body
        .iter()
        .position(|&byte| byte == SET_RULE)
        .expect("set_rule");

    assert_eq!(be_i32(body, set_rule + 1), 60_000);
    assert_eq!(be_i32(body, set_rule + 5), 70_000);
}

#[test]
fn hlist_box_leaders_repeat_payloads_for_all_subtypes() {
    let mut page = empty_page(0);
    let leader = LeaderPayload::HList(box_node(10_000, 1_000, 0, vec![rule_node(1_000, 1_000, 0)]));
    page.root = vlist(
        30_000,
        0,
        0,
        vec![
            hlist(
                30_000,
                1_000,
                0,
                vec![leader_glue(GlueKind::Leaders, 30_000, leader.clone())],
            ),
            hlist(
                30_000,
                1_000,
                0,
                vec![leader_glue(GlueKind::Cleaders, 30_000, leader.clone())],
            ),
            hlist(
                30_000,
                1_000,
                0,
                vec![leader_glue(GlueKind::Xleaders, 30_000, leader)],
            ),
        ],
    );

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);

    assert_eq!(count_op(body, SET_RULE), 9);
}

#[test]
fn vlist_box_leaders_repeat_payloads_downward() {
    let mut page = empty_page(0);
    let leader = LeaderPayload::HList(box_node(
        1_000,
        10_000,
        0,
        vec![rule_node(1_000, 10_000, 0)],
    ));
    page.root = vlist(
        1_000,
        30_000,
        0,
        vec![leader_glue(GlueKind::Xleaders, 30_000, leader)],
    );

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);

    assert_eq!(count_op(body, SET_RULE), 3);
}

#[test]
fn vlist_rule_leaders_use_glue_height_and_running_width() {
    let mut page = empty_page(0);
    page.root = vlist(
        40_000,
        70_000,
        0,
        vec![PageNode::Glue {
            spec: GlueSpec {
                width: sp(70_000),
                stretch: sp(0),
                stretch_order: GlueOrder::Normal,
                shrink: sp(0),
                shrink_order: GlueOrder::Normal,
            },
            kind: GlueKind::Leaders,
            leader: Some(LeaderPayload::Rule {
                width: None,
                height: Some(sp(1)),
                depth: Some(sp(2)),
            }),
        }],
    );

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);
    let put_rule = body
        .iter()
        .position(|&byte| byte == PUT_RULE)
        .expect("put_rule");

    assert_eq!(be_i32(body, put_rule + 1), 70_000);
    assert_eq!(be_i32(body, put_rule + 5), 40_000);
}

#[test]
fn specials_emit_xxx1_and_xxx4_at_anchor_positions() {
    let mut page = empty_page(0);
    page.root = hlist(
        0,
        5,
        0,
        vec![
            PageNode::WhatsitAnchor { effect_index: 0 },
            PageNode::WhatsitAnchor { effect_index: 1 },
        ],
    );
    page.effects = vec![
        PageEffect::Special {
            class: "dvi".to_owned(),
            payload: b"abc".to_vec(),
        },
        PageEffect::Special {
            class: "dvi".to_owned(),
            payload: vec![b'x'; 256],
        },
    ];

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let body = page_body(&dvi, 16);
    let short = body.iter().position(|&byte| byte == XXX1).expect("xxx1");
    let long = body.iter().position(|&byte| byte == XXX4).expect("xxx4");

    assert_eq!(&body[short..short + 5], &[XXX1, 3, b'a', b'b', b'c']);
    assert_eq!(be_i32(body, long + 1), 256);
    assert_eq!(&body[long + 5..long + 9], b"xxxx");
}

fn glyph_page(count0: i32) -> PageArtifact {
    crate::UnvalidatedPageArtifact {
        job: JobInfo {
            mag: 1200,
            banner: "B".to_owned(),
            h_offset: sp(0),
            v_offset: sp(0),
        },
        fonts: vec![font_resource(3, "cmr10")],
        counts: [count0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        root: hlist(300, 100, 30, vec![char_node(3, b'A' as u32, 50)]),
        effects: Vec::new(),
    }
    .validate()
    .expect("glyph page validates")
}

fn empty_page(count0: i32) -> PageArtifact {
    let mut page = glyph_page(count0);
    page.testing_mut().fonts.clear();
    page.testing_mut().root = hlist(0, 0, 0, Vec::new());
    page
}

fn hlist(width: i32, height: i32, depth: i32, children: Vec<PageNode>) -> PageNode {
    PageNode::HList(box_node(width, height, depth, children))
}

fn vlist(width: i32, height: i32, depth: i32, children: Vec<PageNode>) -> PageNode {
    PageNode::VList(box_node(width, height, depth, children))
}

fn box_node(width: i32, height: i32, depth: i32, children: Vec<PageNode>) -> BoxNode {
    BoxNode {
        width: sp(width),
        height: sp(height),
        depth: sp(depth),
        shift: sp(0),
        glue_set: GlueSetRatio::ZERO,
        glue_sign: GlueSign::Normal,
        glue_order: GlueOrder::Normal,
        children,
    }
}

fn char_node(font_id: u32, ch: u32, width: i32) -> PageNode {
    PageNode::Char {
        font_id,
        ch,
        width: sp(width),
    }
}

fn kern_node(amount: i32) -> PageNode {
    PageNode::Kern {
        amount: sp(amount),
        kind: crate::KernKind::Explicit,
    }
}

fn leader_glue(kind: GlueKind, width: i32, leader: LeaderPayload) -> PageNode {
    PageNode::Glue {
        spec: GlueSpec {
            width: sp(width),
            stretch: sp(0),
            stretch_order: GlueOrder::Normal,
            shrink: sp(0),
            shrink_order: GlueOrder::Normal,
        },
        kind,
        leader: Some(leader),
    }
}

fn rule_node(width: i32, height: i32, depth: i32) -> PageNode {
    PageNode::Rule {
        width: Some(sp(width)),
        height: Some(sp(height)),
        depth: Some(sp(depth)),
    }
}

fn count_op(bytes: &[u8], op: u8) -> usize {
    bytes.iter().filter(|&&byte| byte == op).count()
}

fn font_resource(font_id: u32, name: &str) -> FontResource {
    FontResource {
        font_id,
        name: name.to_owned(),
        tfm_content_hash: ContentHash::from_bytes(name.as_bytes()),
        tfm_checksum: 0x1234_5678,
        design_size: sp(655_360),
        at_size: sp(655_360),
    }
}

fn sp(value: i32) -> Scaled {
    Scaled::from_raw(value)
}

fn page_body(dvi: &[u8], bop: usize) -> &[u8] {
    let start = bop + 45;
    let end = page_eop(dvi, bop);
    &dvi[start..end]
}

fn page_eop(dvi: &[u8], bop: usize) -> usize {
    let start = bop + 45;
    start
        + dvi[start..]
            .iter()
            .position(|&byte| byte == EOP)
            .expect("eop")
}

fn font_def_bytes(number: u8, name: &str) -> Vec<u8> {
    let mut bytes = vec![FNT_DEF1, number];
    bytes.extend_from_slice(&0x1234_5678_u32.to_be_bytes());
    bytes.extend_from_slice(&655_360_i32.to_be_bytes());
    bytes.extend_from_slice(&655_360_i32.to_be_bytes());
    bytes.push(0);
    bytes.push(name.len() as u8);
    bytes.extend_from_slice(name.as_bytes());
    bytes
}

fn assert_font_def(dvi: &[u8], offset: usize, number: u8, name: &str) {
    assert_eq!(
        &dvi[offset..offset + 16 + name.len()],
        font_def_bytes(number, name)
    );
}

fn find_font_def(dvi: &[u8], name: &[u8], start: usize) -> Option<usize> {
    dvi[start..]
        .windows(name.len())
        .position(|window| window == name)
        .map(|name_pos| start + name_pos - 16)
}

fn be_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn be_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}
