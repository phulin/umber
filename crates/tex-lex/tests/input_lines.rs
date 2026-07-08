use std::fs;

use tex_lex::{FileInput, LineEvent, LineReader, MemoryInput};
use tex_state::Universe;
use tex_state::env::banks::IntParam;

#[test]
#[allow(clippy::disallowed_methods)] // host-side fixture setup, not engine I/O
fn memory_and_file_sources_share_tex_line_handling() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, b'!' as i32);

    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("input.tex");
    fs::write(&path, "abc  \r\n   \r\ndef").expect("write test fixture");

    let mut memory = LineReader::new(MemoryInput::new("abc  \r\n   \r\ndef"));
    let file_handle = fs::File::open(&path).expect("open test fixture");
    let mut file = LineReader::new(FileInput::from_file(file_handle));

    let mut memory_events = Vec::new();
    while let Some(event) = memory
        .next_event(&stores)
        .expect("memory input should read")
    {
        memory_events.push(event);
    }

    let mut file_events = Vec::new();
    while let Some(event) = file.next_event(&stores).expect("file input should read") {
        file_events.push(event);
    }

    assert_eq!(
        memory_events,
        vec![
            LineEvent::Text("abc!".to_owned()),
            LineEvent::Par,
            LineEvent::Text("def!".to_owned()),
        ]
    );
    assert_eq!(file_events, memory_events);
}

#[test]
fn inactive_endlinechar_keeps_blank_physical_lines_from_becoming_par_events() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);

    let mut reader = LineReader::new(MemoryInput::new("a\n   \nb"));

    let mut events = Vec::new();
    while let Some(event) = reader
        .next_event(&stores)
        .expect("memory input should read")
    {
        events.push(event);
    }

    assert_eq!(
        events,
        vec![
            LineEvent::Text("a".to_owned()),
            LineEvent::Text(String::new()),
            LineEvent::Text("b".to_owned()),
        ]
    );
}
