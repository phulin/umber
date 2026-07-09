use super::{DviFile, command_at_or_before, disassemble_page};

#[test]
fn recovers_pages_from_backpointer_chain() {
    let bytes = two_page_dvi();

    let file = DviFile::parse(&bytes).expect("parse DVI backpointers");

    assert_eq!(file.pages.len(), 2);
    assert_eq!(file.pages[0].index, 0);
    assert_eq!(file.pages[1].index, 1);
    assert_eq!(file.pages[0].counts[0], 1);
    assert_eq!(file.pages[1].counts[0], 2);
    assert_eq!(
        file.page_for_offset(file.pages[1].bop_offset),
        Some(&file.pages[1])
    );
}

#[test]
fn disassembles_selected_page_only() {
    let bytes = two_page_dvi();

    let page = disassemble_page(&bytes, 1).expect("disassemble second page");

    assert!(page.contains("page 2 count0=2"));
    assert!(page.contains("right4 42"));
    assert!(page.contains("setchar65"));
    assert!(!page.contains("count0=1"));
}

#[test]
fn finds_command_owning_operand_offset() {
    let bytes = two_page_dvi();
    let file = DviFile::parse(&bytes).expect("parse DVI backpointers");
    let right4_offset = file.pages[1].bop_offset + 45;

    let command =
        command_at_or_before(&bytes, 1, right4_offset + 3).expect("find command at operand");

    let command = command.expect("operand belongs to a command");
    assert_eq!(command.name, "right4");
    assert_eq!(command.offset, right4_offset);
}

fn two_page_dvi() -> Vec<u8> {
    let mut bytes = Vec::new();
    preamble(&mut bytes);
    let first_bop = bytes.len();
    page(&mut bytes, 1, -1, &[]);
    let second_bop = bytes.len();
    page(
        &mut bytes,
        2,
        i32::try_from(first_bop).expect("small test offset"),
        &[146, 0, 0, 0, 42, 65],
    );
    postamble(
        &mut bytes,
        i32::try_from(second_bop).expect("small test offset"),
        2,
    );
    bytes
}

fn preamble(bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&[247, 2]);
    bytes.extend_from_slice(&25_400_000i32.to_be_bytes());
    bytes.extend_from_slice(&473_628_672i32.to_be_bytes());
    bytes.extend_from_slice(&1000i32.to_be_bytes());
    bytes.push(4);
    bytes.extend_from_slice(b"test");
}

fn page(bytes: &mut Vec<u8>, count0: i32, previous: i32, body: &[u8]) {
    bytes.push(139);
    bytes.extend_from_slice(&count0.to_be_bytes());
    for _ in 1..10 {
        bytes.extend_from_slice(&0i32.to_be_bytes());
    }
    bytes.extend_from_slice(&previous.to_be_bytes());
    bytes.extend_from_slice(body);
    bytes.push(140);
}

fn postamble(bytes: &mut Vec<u8>, final_bop: i32, pages: u16) {
    let post = bytes.len();
    bytes.push(248);
    bytes.extend_from_slice(&final_bop.to_be_bytes());
    bytes.extend_from_slice(&25_400_000i32.to_be_bytes());
    bytes.extend_from_slice(&473_628_672i32.to_be_bytes());
    bytes.extend_from_slice(&1000i32.to_be_bytes());
    bytes.extend_from_slice(&0i32.to_be_bytes());
    bytes.extend_from_slice(&0i32.to_be_bytes());
    bytes.extend_from_slice(&1u16.to_be_bytes());
    bytes.extend_from_slice(&pages.to_be_bytes());
    bytes.push(249);
    bytes.extend_from_slice(
        &u32::try_from(post)
            .expect("small test offset")
            .to_be_bytes(),
    );
    bytes.push(2);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(223);
    }
    bytes.extend_from_slice(&[223, 223, 223, 223]);
}
