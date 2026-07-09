#![allow(clippy::disallowed_methods)] // host tool, not engine code

use anyhow::Result;
use refexec::{DviComparison, compare_dvi_bytes};

#[test]
fn dvi_compare_normalizes_only_preamble_comment_payload() -> Result<()> {
    let mut left = vec![247, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 232, 3];
    left.extend_from_slice(b"abc");
    left.extend_from_slice(&[139, 140]);
    let mut right = vec![247, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 232, 3];
    right.extend_from_slice(b"xyz");
    right.extend_from_slice(&[139, 140]);

    assert_eq!(compare_dvi_bytes(&left, &right)?, DviComparison::Equal);

    right[18] = 141;
    let DviComparison::Different(diff) = compare_dvi_bytes(&left, &right)? else {
        panic!("body byte mismatch should be reported");
    };
    assert_eq!(diff.offset, 18);
    Ok(())
}
