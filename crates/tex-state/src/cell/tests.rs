use super::{BankTag, CellId, GLOBAL_SHIFT};

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
    ];

    for bank in banks {
        let local = CellId::new(bank, 32_767);
        assert_eq!(local.bank(), bank);
        assert_eq!(local.index(), 32_767);
        assert!(!local.is_global());

        let global = CellId::new_global(bank, 32_767);
        assert_eq!(global.bank(), bank);
        assert_eq!(global.index(), 32_767);
        assert!(global.is_global());
        assert_eq!(global.raw(), local.raw() | (1 << GLOBAL_SHIFT));
    }
}
