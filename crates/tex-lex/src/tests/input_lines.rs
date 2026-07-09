use crate::{LineEvent, LineReader, MemoryInput, WorldInput};
use tex_state::env::banks::IntParam;
use tex_state::{Universe, World};

#[test]
fn memory_and_file_sources_share_tex_line_handling() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, b'!' as i32);

    let mut world = World::memory();
    world
        .set_memory_file("input.tex", b"abc  \r\n   \r\ndef".to_vec())
        .expect("seed memory world");
    let content = world.read_file("input.tex").expect("read memory fixture");

    let mut memory = LineReader::new(MemoryInput::new("abc  \r\n   \r\ndef"));
    let mut file = LineReader::new(WorldInput::from_content(content));

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
            LineEvent::Text("!".to_owned()),
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
