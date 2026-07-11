use crate::metrics::{
    CharMetrics, CharTag, FontMetrics, FontMetricsValidationError, LigKernChar, LigKernCommand,
    LigKernInstruction, MAX_LIG_KERN_PROGRAM_LEN,
};
use tex_arith::Scaled;

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
