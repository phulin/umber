use super::{BANK_SHIFT, BankTag, CellId, GLOBAL_SHIFT, INDEX_MASK};

#[test]
fn cell_id_packs_every_bank_index_and_global_bit() {
    let banks = [
        BankTag::Meaning,
        BankTag::Count,
        BankTag::Dimen,
        BankTag::Skip,
        BankTag::Toks,
        BankTag::Box,
        BankTag::Muskip,
        BankTag::IntParam,
        BankTag::DimenParam,
        BankTag::GlueParam,
        BankTag::TokParam,
        BankTag::FontDimen,
        BankTag::FontParamLen,
        BankTag::FontHyphenChar,
        BankTag::FontSkewChar,
        BankTag::CurrentFont,
        BankTag::MathFamilyFont,
        BankTag::PdfLpCode,
        BankTag::PdfRpCode,
        BankTag::PdfEfCode,
        BankTag::PdfTagCode,
        BankTag::PdfKnbsCode,
        BankTag::PdfStbsCode,
        BankTag::PdfShbsCode,
        BankTag::PdfKnbcCode,
        BankTag::PdfKnacCode,
        BankTag::PdfNoLigatures,
    ];

    for bank in banks {
        for index in [32_767, 1 << 26, INDEX_MASK] {
            let local = CellId::new(bank, index);
            assert_eq!(local.bank(), bank);
            assert_eq!(local.index(), index);
            assert!(!local.is_global());
            assert_eq!(CellId::from_raw(local.raw()), Some(local));

            let global = CellId::new_global(bank, index);
            assert_eq!(global.bank(), bank);
            assert_eq!(global.index(), index);
            assert!(global.is_global());
            assert_eq!(global.raw(), local.raw() | (1_u64 << GLOBAL_SHIFT));
            assert_eq!(CellId::from_raw(global.raw()), Some(global));
        }
    }
}

#[test]
fn detached_cell_decode_rejects_reserved_bank_bits() {
    assert_eq!(CellId::from_raw(27_u64 << BANK_SHIFT), None);
    assert_eq!(CellId::from_raw(u64::MAX), None);
}
