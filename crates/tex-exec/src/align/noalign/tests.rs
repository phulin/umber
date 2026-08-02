use super::*;
use crate::{CanonicalMainControl, MainControlStep};
use std::sync::Arc;
use tex_command::{RegisteredSourceKind, SourceRegistration};

#[test]
fn noalign_close_resumes_row_lookahead_in_both_modes() {
    for primitive in ["\\halign", "\\valign"] {
        let mut stores = Universe::new_with_plain_catcodes();
        stores.set_count(0, 3);
        let mut control = CanonicalMainControl::tex82_initex(&mut stores);
        let source = format!("{primitive}{{#\\cr\\noalign{{\\count0=7}}x\\cr}}\\end");
        control
            .register_root_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(source.into_bytes()),
            ))
            .expect("register canonical noalign source");
        for _ in 0..128 {
            match control
                .step(&mut stores)
                .expect("canonical noalign program executes")
            {
                MainControlStep::End | MainControlStep::EndOfInput => break,
                MainControlStep::Continue => {}
            }
        }

        assert_eq!(stores.count(0), 3, "local noalign assignments restore");
        assert!(
            control.active_alignment().is_none(),
            "the row after noalign must be consumed and the alignment finished"
        );
    }
}
