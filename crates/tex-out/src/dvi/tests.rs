use super::{DEN, EOP, FNT_DEF1, ID_BYTE, NUM, POST, POST_POST, PRE, write_dvi};
use crate::{
    BoxNode, ContentHash, FontResource, GlueOrder, GlueSetRatio, GlueSign, JobInfo, PageArtifact,
    PageNode,
};
use tex_arith::Scaled;

#[test]
fn writes_preamble_bop_font_defs_and_postamble() {
    let dvi = write_dvi(&[glyph_page(7)]).expect("DVI writes");
    let pre_len = 16;
    let bop = pre_len;
    let post = 83;
    let post_post = 133;

    assert_eq!(dvi[0], PRE);
    assert_eq!(dvi[1], ID_BYTE);
    assert_eq!(be_i32(&dvi, 2), NUM);
    assert_eq!(be_i32(&dvi, 6), DEN);
    assert_eq!(be_i32(&dvi, 10), 1200);
    assert_eq!(dvi[14], 1);
    assert_eq!(dvi[15], b'B');

    assert_eq!(dvi[bop], 139);
    assert_eq!(be_i32(&dvi, bop + 1), 7);
    assert_eq!(be_i32(&dvi, bop + 41), -1);

    assert_font_def(&dvi, 61, 0);
    assert_eq!(dvi[82], EOP);

    assert_eq!(dvi[post], POST);
    assert_eq!(be_i32(&dvi, post + 1), bop as i32);
    assert_eq!(be_i32(&dvi, post + 5), NUM);
    assert_eq!(be_i32(&dvi, post + 9), DEN);
    assert_eq!(be_i32(&dvi, post + 13), 1200);
    assert_eq!(be_i32(&dvi, post + 17), 230);
    assert_eq!(be_i32(&dvi, post + 21), 300);
    assert_eq!(be_u16(&dvi, post + 25), 0);
    assert_eq!(be_u16(&dvi, post + 27), 1);

    assert_font_def(&dvi, post + 29, 0);
    assert_eq!(dvi[post_post], POST_POST);
    assert_eq!(be_i32(&dvi, post_post + 1), post as i32);
    assert_eq!(dvi[post_post + 5], ID_BYTE);
    assert!(dvi[post_post + 6..].iter().all(|&byte| byte == 223));
    assert!(dvi[post_post + 6..].len() >= 4);
    assert_eq!(dvi.len() % 4, 0);
}

#[test]
fn chains_bop_pointers_across_pages() {
    let dvi = write_dvi(&[empty_page(1), empty_page(2)]).expect("DVI writes");
    let first_bop = 16;
    let second_bop = 62;
    let post = 108;

    assert_eq!(dvi[first_bop], 139);
    assert_eq!(be_i32(&dvi, first_bop + 1), 1);
    assert_eq!(be_i32(&dvi, first_bop + 41), -1);
    assert_eq!(dvi[second_bop], 139);
    assert_eq!(be_i32(&dvi, second_bop + 1), 2);
    assert_eq!(be_i32(&dvi, second_bop + 41), first_bop as i32);
    assert_eq!(dvi[post], POST);
    assert_eq!(be_i32(&dvi, post + 1), second_bop as i32);
    assert_eq!(be_u16(&dvi, post + 27), 2);
}

#[test]
fn defines_each_font_at_first_use_and_repeats_used_fonts_in_postamble() {
    let mut page = glyph_page(0);
    page.fonts.push(FontResource {
        font_id: 9,
        name: "cmtt10".to_owned(),
        tfm_content_hash: ContentHash::from_bytes(b"cmtt10.tfm"),
        tfm_checksum: 0x1111_2222,
        design_size: Scaled::from_raw(655_360),
        at_size: Scaled::from_raw(655_360),
    });
    let PageNode::VList(box_node) = &mut page.root else {
        panic!("sample page root should be vlist");
    };
    box_node.children.push(PageNode::Char {
        font_id: 9,
        ch: 'B' as u32,
    });

    let dvi = write_dvi(&[page]).expect("DVI writes");
    let first_cmr = find_font_def(&dvi, b"cmr10", 0).expect("body cmr10 def");
    let first_cmtt = find_font_def(&dvi, b"cmtt10", first_cmr + 1).expect("body cmtt10 def");
    assert_eq!(dvi[first_cmr], FNT_DEF1);
    assert_eq!(dvi[first_cmr + 1], 0);
    assert_eq!(dvi[first_cmtt], FNT_DEF1);
    assert_eq!(dvi[first_cmtt + 1], 1);

    let post = dvi.iter().position(|&byte| byte == POST).expect("post");
    let post_cmr = find_font_def(&dvi, b"cmr10", post).expect("post cmr10 def");
    let post_cmtt = find_font_def(&dvi, b"cmtt10", post_cmr + 1).expect("post cmtt10 def");
    assert!(post_cmr < post_cmtt);
}

fn glyph_page(count0: i32) -> PageArtifact {
    PageArtifact {
        job: JobInfo {
            mag: 1200,
            banner: "B".to_owned(),
        },
        fonts: vec![FontResource {
            font_id: 3,
            name: "cmr10".to_owned(),
            tfm_content_hash: ContentHash::from_bytes(b"cmr10.tfm"),
            tfm_checksum: 0x1234_5678,
            design_size: Scaled::from_raw(655_360),
            at_size: Scaled::from_raw(655_360),
        }],
        counts: [count0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        root: PageNode::VList(BoxNode {
            width: Scaled::from_raw(300),
            height: Scaled::from_raw(200),
            depth: Scaled::from_raw(30),
            shift: Scaled::from_raw(0),
            glue_set: GlueSetRatio { raw: 0 },
            glue_sign: GlueSign::Normal,
            glue_order: GlueOrder::Normal,
            children: vec![PageNode::Char {
                font_id: 3,
                ch: 'A' as u32,
            }],
        }),
        effects: Vec::new(),
    }
}

fn empty_page(count0: i32) -> PageArtifact {
    let mut page = glyph_page(count0);
    page.fonts.clear();
    if let PageNode::VList(box_node) = &mut page.root {
        box_node.children.clear();
    }
    page
}

fn assert_font_def(dvi: &[u8], offset: usize, number: u8) {
    assert_eq!(dvi[offset], FNT_DEF1);
    assert_eq!(dvi[offset + 1], number);
    assert_eq!(be_u32(dvi, offset + 2), 0x1234_5678);
    assert_eq!(be_i32(dvi, offset + 6), 655_360);
    assert_eq!(be_i32(dvi, offset + 10), 655_360);
    assert_eq!(dvi[offset + 14], 0);
    assert_eq!(dvi[offset + 15], 5);
    assert_eq!(&dvi[offset + 16..offset + 21], b"cmr10");
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

fn be_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn be_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}
