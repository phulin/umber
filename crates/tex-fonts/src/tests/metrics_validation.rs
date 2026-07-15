use crate::metrics::{
    CharMetrics, CharTag, FontConstruction, FontMetrics, FontMetricsValidationError, LigKernChar,
    LigKernCommand, LigKernInstruction, LoadedFont, MAX_LIG_KERN_PROGRAM_LEN,
};
use tex_arith::Scaled;

#[test]
fn generated_fonts_preserve_source_ancestry_and_pdftex_rounding() {
    let mut characters = vec![None; 256];
    characters[b'A' as usize] = Some(CharMetrics {
        width: Scaled::from_raw(500_000),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        italic_correction: Scaled::from_raw(0),
        tag: CharTag::None,
    });
    let source = LoadedFont::new(
        "test",
        "/host/path/test.tfm",
        [7; 32],
        42,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(12 * Scaled::UNITY),
        vec![
            Scaled::from_raw(0),
            Scaled::from_raw(4 * Scaled::UNITY),
            Scaled::from_raw(0),
            Scaled::from_raw(0),
            Scaled::from_raw(0),
            Scaled::from_raw(12 * Scaled::UNITY),
            Scaled::from_raw(0),
        ],
        FontMetrics::new(characters, Vec::new(), None, None, Vec::new()),
    );
    let copied = source.copied(vec![Scaled::from_raw(9 * Scaled::UNITY); 8]);
    let letterspaced = copied
        .letterspaced(Scaled::from_raw(99), 100, true)
        .expect("bounded metric derivation");

    assert_eq!(copied.parameters().len(), 8);
    assert_eq!(letterspaced.name(), "test+100ls");
    assert_eq!(letterspaced.parameters()[1].raw(), 4 * Scaled::UNITY);
    assert_eq!(
        letterspaced
            .metrics()
            .character(b'A')
            .expect("letterspaced character remains present")
            .width
            .raw(),
        500_000 + 78_643
    );
    let FontConstruction::Letterspaced {
        source: ancestry,
        amount: 100,
        no_ligatures: true,
    } = letterspaced.construction()
    else {
        panic!("letterspaced construction metadata")
    };
    assert_eq!(*ancestry, copied.source_identity());
    assert_ne!(source.source_identity(), copied.source_identity());

    let expanded = source.expanded(100);
    let expanded_metrics = expanded
        .metrics()
        .character(b'A')
        .expect("expanded character remains present");
    assert_eq!(expanded_metrics.width.raw(), 550_000);
    assert_eq!(expanded_metrics.height.raw(), 0);
    assert_eq!(expanded.parameters(), source.parameters());
    assert!(matches!(
        expanded.construction(),
        FontConstruction::Expanded { ratio: 100, .. }
    ));
}

#[test]
fn lig_kern_program_capacity_accepts_both_addressable_length_edges() {
    for (len, start) in [
        (usize::from(u16::MAX), u16::MAX - 1),
        (MAX_LIG_KERN_PROGRAM_LEN, u16::MAX),
    ] {
        let metrics = metrics_with_program(len, start, None);
        metrics.validate().expect("addressable program validates");
        let step = metrics
            .lig_kern_iter(LigKernChar::Char(b'A'), LigKernChar::Char(b'A'))
            .next()
            .expect("start instruction is addressable");
        assert_eq!(step.instruction_index, start);
    }
}

#[test]
fn lig_kern_program_capacity_rejects_first_unaddressable_length() {
    let metrics = metrics_with_program(MAX_LIG_KERN_PROGRAM_LEN + 1, 0, None);
    assert_eq!(
        metrics.validate(),
        Err(FontMetricsValidationError::LigKernProgramTooLong {
            len: MAX_LIG_KERN_PROGRAM_LEN + 1,
            max: MAX_LIG_KERN_PROGRAM_LEN,
        })
    );
}

#[test]
fn lig_kern_cursor_reaches_u16_max_without_overflow() {
    let metrics = metrics_with_program(
        MAX_LIG_KERN_PROGRAM_LEN,
        u16::MAX - 1,
        Some(usize::from(u16::MAX - 1)),
    );
    metrics
        .validate()
        .expect("final cursor transition validates");
    let indices: Vec<_> = metrics
        .lig_kern_iter(LigKernChar::Char(b'A'), LigKernChar::Char(b'A'))
        .map(|step| step.instruction_index)
        .collect();
    assert_eq!(indices, [u16::MAX - 1, u16::MAX]);
}

#[test]
fn unvalidated_lig_kern_cursor_stops_instead_of_wrapping() {
    let metrics = metrics_with_program(
        MAX_LIG_KERN_PROGRAM_LEN,
        u16::MAX,
        Some(usize::from(u16::MAX)),
    );
    assert_eq!(
        metrics.validate(),
        Err(FontMetricsValidationError::LigKernSkipOutOfBounds {
            instruction: usize::from(u16::MAX),
            target: MAX_LIG_KERN_PROGRAM_LEN,
            len: MAX_LIG_KERN_PROGRAM_LEN,
        })
    );

    let mut iter = metrics.lig_kern_iter(LigKernChar::Char(b'A'), LigKernChar::Char(b'A'));
    assert_eq!(
        iter.next()
            .expect("final instruction is yielded")
            .instruction_index,
        u16::MAX
    );
    assert!(
        iter.next().is_none(),
        "cursor overflow must terminate, not wrap"
    );
}

fn metrics_with_program(
    len: usize,
    start: u16,
    advancing_instruction: Option<usize>,
) -> FontMetrics {
    let stop = LigKernInstruction {
        skip_byte: 128,
        next_char: b'A',
        command: Some(LigKernCommand::Kern(Scaled::from_raw(0))),
    };
    let mut program = vec![stop; len];
    if let Some(index) = advancing_instruction {
        program[index].skip_byte = 0;
    }
    let mut characters = vec![None; 256];
    characters[usize::from(b'A')] = Some(CharMetrics {
        width: Scaled::from_raw(1),
        height: Scaled::from_raw(0),
        depth: Scaled::from_raw(0),
        italic_correction: Scaled::from_raw(0),
        tag: CharTag::LigKern {
            program_index: 0,
            start_index: start,
        },
    });
    FontMetrics::new(characters, program, None, None, Vec::new())
}
