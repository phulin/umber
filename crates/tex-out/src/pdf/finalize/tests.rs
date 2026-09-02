use super::*;

#[test]
fn type1_descriptor_fallback_uses_named_tfm_characters_instead_of_table_extrema() {
    let mut widths = [Scaled::from_raw(0); 256];
    let mut heights = [Scaled::from_raw(0); 256];
    let mut depths = [Scaled::from_raw(0); 256];
    widths[usize::from(b'.')] = Scaled::from_raw(156_000);
    widths[usize::from(b',')] = Scaled::from_raw(9 * Scaled::UNITY);
    heights[usize::from(b'h')] = Scaled::from_raw(7 * Scaled::UNITY);
    heights[usize::from(b'H')] = Scaled::from_raw(6 * Scaled::UNITY);
    heights[usize::from(b'A')] = Scaled::from_raw(9 * Scaled::UNITY);
    depths[usize::from(b'y')] = Scaled::from_raw(2 * Scaled::UNITY);
    depths[usize::from(b'g')] = Scaled::from_raw(3 * Scaled::UNITY);
    let metrics = PdfFontMetricsInput {
        widths,
        heights,
        depths,
        x_height: Scaled::from_raw(4 * Scaled::UNITY),
    };

    assert_eq!(
        type1_fallback_descriptor_metrics(&metrics, Scaled::from_raw(10 * Scaled::UNITY),),
        [700, -200, 600, 79, 400],
    );
}

#[test]
fn type1_std_vw_overrides_the_period_width_fallback() {
    fn type1_program(header: &[u8]) -> tex_fonts::PdfType1Program {
        let mut pfb = vec![0x80, 1];
        pfb.extend_from_slice(&(header.len() as u32).to_le_bytes());
        pfb.extend_from_slice(header);
        pfb.extend_from_slice(&[0x80, 2, 1, 0, 0, 0, 0, 0x80, 3]);
        tex_fonts::PdfType1Program::from_pfb(&pfb).expect("valid synthetic Type-1 program")
    }

    let explicit = type1_program(b"%!PS\n/StdVW [71] def\n");
    let absent = type1_program(b"%!PS\n/ItalicAngle 0 def\n");

    assert_eq!(type1_descriptor_stem_v(&explicit, 79), 71);
    assert_eq!(type1_descriptor_stem_v(&absent, 79), 79);
}
