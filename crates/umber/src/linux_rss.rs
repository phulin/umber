//! Checked Linux resident-set measurement shared by format construction guards.

use std::path::Path;

#[allow(
    clippy::disallowed_methods,
    reason = "native format guard reads Linux process accounting outside deterministic engine state"
)]
pub(crate) fn resident_bytes(path: &Path) -> Option<u64> {
    let statm = std::fs::read_to_string(path).ok()?;
    resident_bytes_from_statm(&statm)
}

pub(crate) fn resident_bytes_from_statm(statm: &str) -> Option<u64> {
    resident_bytes_with_page_size(statm, runtime_page_size()?)
}

#[allow(
    clippy::disallowed_methods,
    reason = "native format guard reads Linux process configuration outside deterministic engine state"
)]
fn runtime_page_size() -> Option<u64> {
    page_size_from_auxv(&std::fs::read("/proc/self/auxv").ok()?)
}

fn page_size_from_auxv(auxv: &[u8]) -> Option<u64> {
    const AT_PAGESZ: usize = 6;
    let word_bytes = size_of::<usize>();
    let entry_bytes = word_bytes.checked_mul(2)?;
    for entry in auxv.chunks_exact(entry_bytes) {
        let (tag, value) = entry.split_at(word_bytes);
        let tag = usize::from_ne_bytes(tag.try_into().ok()?);
        let value = usize::from_ne_bytes(value.try_into().ok()?);
        if tag == AT_PAGESZ {
            return u64::try_from(value).ok().filter(|size| *size > 0);
        }
        if tag == 0 {
            return None;
        }
    }
    None
}

fn resident_bytes_with_page_size(statm: &str, page_size: u64) -> Option<u64> {
    if page_size == 0 {
        return None;
    }
    let resident_pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    resident_pages.checked_mul(page_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_resident_pages_with_supplied_runtime_page_size() {
        assert_eq!(
            resident_bytes_with_page_size("100 3 2 1 0 0 0", 65_536),
            Some(196_608)
        );
    }

    #[test]
    fn rejects_invalid_page_size_and_overflow() {
        assert_eq!(resident_bytes_with_page_size("100 3", 0), None);
        assert_eq!(
            resident_bytes_with_page_size("0 18446744073709551615", 2),
            None
        );
    }

    #[test]
    fn discovers_checked_page_size_from_runtime_auxiliary_vector() {
        let mut auxv = Vec::new();
        auxv.extend_from_slice(&6_usize.to_ne_bytes());
        auxv.extend_from_slice(&65_536_usize.to_ne_bytes());
        assert_eq!(page_size_from_auxv(&auxv), Some(65_536));

        auxv[size_of::<usize>()..].fill(0);
        assert_eq!(page_size_from_auxv(&auxv), None);
        assert_eq!(page_size_from_auxv(&auxv[..auxv.len() - 1]), None);
    }
}
