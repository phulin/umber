use super::*;

struct Builder(Vec<u8>);

impl Builder {
    fn new(size: usize) -> Self {
        Self(vec![0; size])
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn reserve(&mut self, size: usize) -> usize {
        let at = self.len();
        self.0.resize(at + size, 0);
        at
    }

    fn u16(&mut self, at: usize, value: u16) {
        self.0[at..at + 2].copy_from_slice(&value.to_be_bytes());
    }

    fn i16(&mut self, at: usize, value: i16) {
        self.0[at..at + 2].copy_from_slice(&value.to_be_bytes());
    }

    fn u32(&mut self, at: usize, value: u32) {
        self.0[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn relative(&mut self, field: usize, parent: usize, target: usize) {
        self.u16(
            field,
            u16::try_from(target - parent).expect("fixture offset"),
        );
    }

    fn coverage(&mut self, glyphs: &[u16]) -> usize {
        let base = self.reserve(4 + glyphs.len() * 2);
        self.u16(base, 1);
        self.u16(base + 2, glyphs.len() as u16);
        for (index, glyph) in glyphs.iter().copied().enumerate() {
            self.u16(base + 4 + index * 2, glyph);
        }
        base
    }

    fn math_values(&mut self, glyph: u16, value: i16) -> usize {
        let base = self.reserve(8);
        self.u16(base + 2, 1);
        self.i16(base + 4, value);
        let coverage = self.coverage(&[glyph]);
        self.relative(base, base, coverage);
        base
    }
}

fn complete_table() -> Vec<u8> {
    let mut out = Builder::new(10);
    out.u32(0, 0x0001_0000);

    let constants = out.reserve(214);
    out.relative(4, 0, constants);
    out.i16(constants, 80);
    out.i16(constants + 2, 60);
    out.u16(constants + 4, 1_300);
    out.u16(constants + 6, 1_500);
    out.i16(constants + 8, 12);
    out.i16(constants + 212, 55);

    let glyph_info = out.reserve(8);
    out.relative(6, 0, glyph_info);
    let italic = out.math_values(1, 41);
    out.relative(glyph_info, glyph_info, italic);
    let accent = out.math_values(2, 222);
    out.relative(glyph_info + 2, glyph_info, accent);
    let extended = out.coverage(&[3]);
    out.relative(glyph_info + 4, glyph_info, extended);

    let kern_infos = out.reserve(12);
    out.u16(kern_infos + 2, 1);
    out.relative(glyph_info + 6, glyph_info, kern_infos);
    let kern_coverage = out.coverage(&[4]);
    out.relative(kern_infos, kern_infos, kern_coverage);
    let kern = out.reserve(14);
    out.u16(kern, 1);
    out.i16(kern + 2, 100);
    out.i16(kern + 6, -10);
    out.i16(kern + 10, -20);
    out.relative(kern_infos + 4, kern_infos, kern);

    let variants = out.reserve(14);
    out.relative(8, 0, variants);
    out.u16(variants, 20);
    out.u16(variants + 6, 1);
    out.u16(variants + 8, 1);
    let vertical_coverage = out.coverage(&[5]);
    out.relative(variants + 2, variants, vertical_coverage);
    let horizontal_coverage = out.coverage(&[6]);
    out.relative(variants + 4, variants, horizontal_coverage);

    let vertical = out.reserve(8);
    out.u16(vertical + 2, 1);
    out.u16(vertical + 4, 7);
    out.u16(vertical + 6, 800);
    out.relative(variants + 10, variants, vertical);
    let assembly = out.reserve(16);
    out.i16(assembly, 9);
    out.u16(assembly + 4, 1);
    out.u16(assembly + 6, 8);
    out.u16(assembly + 8, 10);
    out.u16(assembly + 10, 10);
    out.u16(assembly + 12, 400);
    out.u16(assembly + 14, 1);
    out.relative(vertical, vertical, assembly);

    let horizontal = out.reserve(8);
    out.u16(horizontal + 2, 1);
    out.u16(horizontal + 4, 9);
    out.u16(horizontal + 6, 900);
    out.relative(variants + 12, variants, horizontal);

    let device = out.reserve(8);
    out.u16(device, 10);
    out.u16(device + 2, 11);
    out.u16(device + 4, 1);
    out.u16(device + 6, 0x7000); // +1, -1 in two-bit packed form.
    out.relative(constants + 10, constants, device);
    out.0
}

#[test]
fn parses_every_math_subtable_losslessly() {
    let table = parse_math(&complete_table(), 16, 1_000, 100).expect("complete MATH table");
    assert_eq!(table.constants.script_percent_scale_down, 80);
    assert_eq!(table.constants.value(MathConstant::MathLeading).value, 12);
    assert_eq!(
        table.constants.value(MathConstant::MathLeading).adjustment,
        Some(MathAdjustment::Device {
            start_size: 10,
            end_size: 11,
            delta_format: 1,
            deltas: vec![1, -1],
        })
    );
    let info = table.glyph_info.expect("glyph info");
    assert_eq!(info.italic_corrections[&1].value, 41);
    assert_eq!(info.top_accent_attachments[&2].value, 222);
    assert!(info.extended_shapes.contains(&3));
    let kern = info.kern_info[&4].top_right.as_ref().expect("kern");
    assert_eq!(kern.correction_heights[0].value, 100);
    assert_eq!(kern.kern_values.len(), 2);
    let variants = table.variants.expect("variants");
    assert_eq!(variants.min_connector_overlap, 20);
    assert_eq!(variants.vertical[&5].variants[0].glyph_id, 7);
    let part = variants.vertical[&5]
        .assembly
        .as_ref()
        .expect("assembly")
        .parts[0];
    assert_eq!(part.glyph_id, 8);
    assert!(part.extender);
    assert_eq!(variants.horizontal[&6].variants[0].glyph_id, 9);
}

#[test]
fn rejects_malformed_offsets_and_mismatched_coverage() {
    let mut bad_offset = complete_table();
    bad_offset[4..6].copy_from_slice(&u16::MAX.to_be_bytes());
    assert!(matches!(
        parse_math(&bad_offset, 16, 1_000, 100),
        Err(FontParseError::InvalidMath(_))
    ));

    let mut mismatch = complete_table();
    let glyph_info = usize::from(u16::from_be_bytes([mismatch[6], mismatch[7]]));
    let italic = glyph_info
        + usize::from(u16::from_be_bytes([
            mismatch[glyph_info],
            mismatch[glyph_info + 1],
        ]));
    let coverage =
        italic + usize::from(u16::from_be_bytes([mismatch[italic], mismatch[italic + 1]]));
    mismatch[coverage + 2..coverage + 4].copy_from_slice(&0_u16.to_be_bytes());
    assert_eq!(
        parse_math(&mismatch, 16, 1_000, 100),
        Err(FontParseError::InvalidMath(
            "MATH coverage/record count mismatch"
        ))
    );
}

#[test]
fn rejects_construction_cycles() {
    let mut cyclic = complete_table();
    let variants = usize::from(u16::from_be_bytes([cyclic[8], cyclic[9]]));
    let vertical = variants
        + usize::from(u16::from_be_bytes([
            cyclic[variants + 10],
            cyclic[variants + 11],
        ]));
    cyclic[vertical..vertical + 2].copy_from_slice(&2_u16.to_be_bytes());
    assert_eq!(
        parse_math(&cyclic, 16, 1_000, 100),
        Err(FontParseError::InvalidMath(
            "cyclic or overlapping MATH offset graph"
        ))
    );
}

#[test]
fn enforces_math_record_and_assembly_limits() {
    assert!(matches!(
        parse_math(&complete_table(), 16, 50, 100),
        Err(FontParseError::LimitExceeded {
            resource: "MATH records",
            ..
        })
    ));
    assert!(matches!(
        parse_math(&complete_table(), 16, 1_000, 0),
        Err(FontParseError::LimitExceeded {
            resource: "MATH assembly parts",
            ..
        })
    ));
}
