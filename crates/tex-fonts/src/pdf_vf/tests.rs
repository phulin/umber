use super::*;

const FIX_ONE: i32 = 1 << 20;

fn preamble() -> Vec<u8> {
    let mut bytes = vec![PRE, VF_ID, 2, b'o', b'k'];
    bytes.extend_from_slice(&0x1234_5678u32.to_be_bytes());
    bytes.extend_from_slice(&(10 * FIX_ONE).to_be_bytes());
    bytes
}

fn append_font(bytes: &mut Vec<u8>, opcode: u8, number: i32, name: &[u8]) {
    bytes.push(opcode);
    let width = usize::from(opcode - FNT_DEF1 + 1);
    bytes.extend_from_slice(&number.to_be_bytes()[4 - width..]);
    bytes.extend_from_slice(&0x0102_0304u32.to_be_bytes());
    bytes.extend_from_slice(&FIX_ONE.to_be_bytes());
    bytes.extend_from_slice(&(10 * FIX_ONE).to_be_bytes());
    bytes.push(0);
    bytes.push(u8::try_from(name.len()).expect("short test name"));
    bytes.extend_from_slice(name);
}

fn append_short_packet(bytes: &mut Vec<u8>, character: u8, width: u32, commands: &[u8]) {
    bytes.push(u8::try_from(commands.len()).expect("short test packet"));
    bytes.push(character);
    bytes.extend_from_slice(&width.to_be_bytes()[1..]);
    bytes.extend_from_slice(commands);
}

fn append_long_packet(bytes: &mut Vec<u8>, character: u32, width: i32, commands: &[u8]) {
    bytes.push(LONG_CHAR);
    bytes.extend_from_slice(
        &u32::try_from(commands.len())
            .expect("test packet length")
            .to_be_bytes(),
    );
    bytes.extend_from_slice(&character.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(commands);
}

fn finish(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.push(POST);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(POST);
    }
    bytes
}

fn one_font_program(commands: &[u8]) -> Vec<u8> {
    let mut bytes = preamble();
    append_font(&mut bytes, FNT_DEF1, 7, b"cmr10");
    append_short_packet(&mut bytes, 65, 0x08_0000, commands);
    finish(bytes)
}

#[test]
fn parses_preamble_local_font_and_typed_short_packet() {
    let mut commands = vec![178, 65, 133, 66, 132];
    commands.extend_from_slice(&FIX_ONE.to_be_bytes());
    commands.extend_from_slice(&(2 * FIX_ONE).to_be_bytes());
    commands.extend_from_slice(&[141, 143, 0xfe, 158, 0, 3, 142, 239, 3]);
    commands.extend_from_slice(b"abc");
    let bytes = one_font_program(&commands);

    let program = VfProgram::parse(&bytes).expect("valid short VF parses");
    assert_eq!(program.comment(), b"ok");
    assert_eq!(program.checksum(), 0x1234_5678);
    assert_eq!(program.design_size(), 10 * FIX_ONE);
    assert_ne!(program.identity().bytes(), [0; 32]);
    assert_eq!(program.local_fonts()[0].logical_name(), b"cmr10");

    let packet = program.packet(65).expect("packet exists");
    assert_eq!(packet.tfm_width, 0x08_0000);
    assert_eq!(packet.metadata.max_stack_depth, 1);
    assert_eq!(
        packet.metadata.character_references,
        [
            VfCharacterReference {
                local_font: 7,
                character: 65,
            },
            VfCharacterReference {
                local_font: 7,
                character: 66,
            },
        ]
    );
    assert!(packet.commands.contains(&VfCommand::Rule {
        height: FIX_ONE,
        width: 2 * FIX_ONE,
        move_cursor: true,
    }));
    assert!(packet.commands.contains(&VfCommand::MoveRight(-2)));
    assert!(packet.commands.contains(&VfCommand::MoveDown(3)));
    assert!(
        packet
            .commands
            .contains(&VfCommand::Special(b"abc".to_vec()))
    );
}

#[test]
fn parses_long_packet_and_all_font_number_widths() {
    let mut bytes = preamble();
    append_font(&mut bytes, 243, 7, b"one");
    append_font(&mut bytes, 243, 200, b"high-byte");
    append_font(&mut bytes, 244, 300, b"two");
    append_font(&mut bytes, 245, 70_000, b"three");
    append_font(&mut bytes, 246, -2, b"four");
    append_long_packet(
        &mut bytes,
        0x102,
        -FIX_ONE,
        &[238, 0xff, 0xff, 0xff, 0xfe, 128, 200],
    );
    let program = VfProgram::parse(&finish(bytes)).expect("valid long VF parses");

    assert_eq!(
        program
            .local_fonts()
            .iter()
            .map(|font| font.number)
            .collect::<Vec<_>>(),
        [7, 200, 300, 70_000, -2]
    );
    let packet = program.packet(0x102).expect("long packet exists");
    assert_eq!(packet.tfm_width, -FIX_ONE);
    assert_eq!(
        packet.metadata.character_references,
        [VfCharacterReference {
            local_font: -2,
            character: 200,
        }]
    );
}

#[test]
fn rejects_truncation_ordering_duplicates_and_bad_commands() {
    assert_eq!(VfProgram::parse(&[]), Err(VfParseError::Truncated));
    assert_eq!(
        VfProgram::parse(&[PRE, 0]),
        Err(VfParseError::InvalidPreamble)
    );

    let mut missing_post = preamble();
    append_font(&mut missing_post, 243, 7, b"font");
    assert_eq!(
        VfProgram::parse(&missing_post),
        Err(VfParseError::MissingPostamble)
    );

    let mut duplicate_font = preamble();
    append_font(&mut duplicate_font, 243, 7, b"one");
    append_font(&mut duplicate_font, 243, 7, b"two");
    assert_eq!(
        VfProgram::parse(&finish(duplicate_font)),
        Err(VfParseError::DuplicateLocalFont(7))
    );

    assert_eq!(
        VfProgram::parse(&one_font_program(&[139])),
        Err(VfParseError::InvalidPacketCommand(139))
    );
    assert_eq!(
        VfProgram::parse(&one_font_program(&[235, 8])),
        Err(VfParseError::UndefinedLocalFont(8))
    );
    assert_eq!(
        VfProgram::parse(&one_font_program(&[142])),
        Err(VfParseError::StackUnderflow)
    );
    assert_eq!(
        VfProgram::parse(&one_font_program(&[141])),
        Err(VfParseError::UnbalancedStack)
    );
}

#[test]
fn rejects_packet_before_font_and_duplicate_character_packets() {
    let mut after_packet = preamble();
    append_short_packet(&mut after_packet, 65, 0, &[]);
    append_font(&mut after_packet, 243, 7, b"late");
    assert_eq!(
        VfProgram::parse(&finish(after_packet)),
        Err(VfParseError::FontDefinitionAfterPacket)
    );

    let mut duplicate = preamble();
    append_font(&mut duplicate, 243, 7, b"font");
    append_short_packet(&mut duplicate, 65, 0, &[]);
    append_short_packet(&mut duplicate, 65, 0, &[]);
    assert_eq!(
        VfProgram::parse(&finish(duplicate)),
        Err(VfParseError::DuplicateCharacter(65))
    );
}

#[test]
fn rejects_noncanonical_postamble_and_packet_truncation() {
    let mut trailing = finish(preamble());
    trailing.push(0);
    assert_eq!(VfProgram::parse(&trailing), Err(VfParseError::TrailingData));

    let mut unpadded = preamble();
    unpadded.push(POST);
    assert!(!unpadded.len().is_multiple_of(4));
    assert_eq!(
        VfProgram::parse(&unpadded),
        Err(VfParseError::InvalidPostamblePadding)
    );

    let mut truncated = preamble();
    append_font(&mut truncated, 243, 7, b"font");
    truncated.extend_from_slice(&[2, 65, 0, 0, 0, 128]);
    assert_eq!(VfProgram::parse(&truncated), Err(VfParseError::Truncated));
}

#[test]
fn enforces_every_configurable_capacity() {
    let bytes = one_font_program(&[65, 239, 2, b'x', b'y']);
    let baseline = VfLimits::default();
    let cases = [
        (
            VfLimits {
                max_input_bytes: bytes.len() - 1,
                ..baseline
            },
            VfParseError::InputTooLarge,
        ),
        (
            VfLimits {
                max_local_fonts: 0,
                ..baseline
            },
            VfParseError::TooManyLocalFonts,
        ),
        (
            VfLimits {
                max_packets: 0,
                ..baseline
            },
            VfParseError::TooManyPackets,
        ),
        (
            VfLimits {
                max_packet_bytes: 4,
                ..baseline
            },
            VfParseError::PacketTooLarge,
        ),
        (
            VfLimits {
                max_total_packet_bytes: 4,
                ..baseline
            },
            VfParseError::TotalPacketBytesExceeded,
        ),
        (
            VfLimits {
                max_total_commands: 1,
                ..baseline
            },
            VfParseError::TooManyCommands,
        ),
        (
            VfLimits {
                max_total_special_bytes: 1,
                ..baseline
            },
            VfParseError::SpecialBytesExceeded,
        ),
    ];
    for (limits, error) in cases {
        assert_eq!(VfProgram::parse_with_limits(&bytes, limits), Err(error));
    }

    let stack_bytes = one_font_program(&[141, 142]);
    assert_eq!(
        VfProgram::parse_with_limits(
            &stack_bytes,
            VfLimits {
                max_stack_depth: 0,
                ..baseline
            },
        ),
        Err(VfParseError::StackOverflow)
    );
}

#[test]
fn rejects_invalid_scaled_size_and_character_without_a_default_font() {
    let mut invalid_size = preamble();
    invalid_size.push(FNT_DEF1);
    invalid_size.push(1);
    invalid_size.extend_from_slice(&0u32.to_be_bytes());
    invalid_size.extend_from_slice(&0i32.to_be_bytes());
    invalid_size.extend_from_slice(&FIX_ONE.to_be_bytes());
    invalid_size.extend_from_slice(&[0, 0]);
    assert_eq!(
        VfProgram::parse(&finish(invalid_size)),
        Err(VfParseError::InvalidLocalFontSize)
    );

    let mut no_font = preamble();
    append_short_packet(&mut no_font, 65, 0, &[65]);
    assert_eq!(
        VfProgram::parse(&finish(no_font)),
        Err(VfParseError::NoCurrentFont)
    );
}
