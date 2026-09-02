use super::*;

#[test]
fn type1_descriptor_fallback_uses_h_and_y_instead_of_table_extrema() {
    let mut heights = [Scaled::from_raw(0); 256];
    let mut depths = [Scaled::from_raw(0); 256];
    heights[usize::from(b'h')] = Scaled::from_raw(7 * Scaled::UNITY);
    heights[usize::from(b'H')] = Scaled::from_raw(6 * Scaled::UNITY);
    heights[usize::from(b'A')] = Scaled::from_raw(9 * Scaled::UNITY);
    depths[usize::from(b'y')] = Scaled::from_raw(2 * Scaled::UNITY);
    depths[usize::from(b'g')] = Scaled::from_raw(3 * Scaled::UNITY);
    let metrics = PdfFontMetricsInput {
        widths: [Scaled::from_raw(0); 256],
        heights,
        depths,
        x_height: Scaled::from_raw(4 * Scaled::UNITY),
    };

    assert_eq!(
        type1_fallback_descriptor_metrics(&metrics, Scaled::from_raw(10 * Scaled::UNITY),),
        [700, -200, 600, 400],
    );
}
