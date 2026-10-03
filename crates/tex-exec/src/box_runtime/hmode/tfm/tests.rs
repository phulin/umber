use super::*;

#[test]
fn physical_replacement_span_stops_before_next_glyph_but_keeps_boundary_kern() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let stores = universe.command_context().expect("test state");
        let mut work = TfmScratch::default();
        for ch in ['a', 'b', 'c'] {
            let provenance = work.append_source(ch, tex_state::token::OriginId::UNKNOWN);
            let glyph = LigatureGlyphCell::new(
                &stores,
                tex_state::font::NULL_FONT,
                ch,
                provenance,
                false,
                false,
                false,
            );
            work.output.push(Some(TfmCell::Glyph(glyph)));
            work.output.push(Some(TfmCell::Kern {
                amount: tex_state::scaled::Scaled::from_raw(1),
                kind: KernKind::Font,
            }));
        }
        let first = 0;
        assert_eq!(work.physical_nodes_through_boundary(first, 0, 1, 1), 2);
        assert_eq!(work.physical_nodes_through_boundary(first, 0, 1, 2), 4);
        assert_eq!(work.physical_nodes_through_boundary(first, 0, 1, 3), 6);
    });
}

#[test]
fn failed_child_replay_preserves_parent_physical_suffix_and_source() {
    crate::test_harness::with_nonstop_plain_universe(|universe| {
        let mut stores = universe.command_context().expect("test state");
        let mut scratch = TfmScratch::default();
        let provenance = scratch.append_source('f', OriginId::UNKNOWN);
        scratch
            .output
            .push(Some(TfmCell::Glyph(LigatureGlyphCell::new(
                &stores,
                tex_state::font::NULL_FONT,
                'f',
                provenance,
                false,
                false,
                false,
            ))));
        scratch.output.push(Some(TfmCell::Kern {
            amount: Scaled::from_raw(17),
            kind: KernKind::Font,
        }));
        let source = ['a', 'b'].map(|ch| crate::mode::PendingHChar {
            font: tex_state::font::NULL_FONT,
            ch,
            origin: OriginId::UNKNOWN,
        });
        let mut output = tex_state::page_node_arena::PageMaterialActiveListBuilder::default();
        stores.open_page_active_list(&mut output);
        let mut sink = super::super::PageNodeSink {
            output: &mut output,
        };
        let mut effects = DiagnosticEffects::new();
        let mut fuel = tex_command::CommandFuelLedger::new(1).expect("bounded fuel");
        let result = run_tfm_ligature_machine_nested_with_work(
            &mut stores,
            &mut effects,
            &source,
            false,
            LigatureRightBoundary::Font,
            false,
            fuel.fuel_mut(),
            &mut scratch,
            &mut sink,
        );
        assert!(result.is_err(), "the child's second pair must exhaust fuel");
        stores.rollback_page_active_list(&mut output);
        assert!(scratch.stack.is_empty());
        assert_eq!(scratch.output.len(), 2);
        assert_eq!(scratch.source(provenance).len(), 1);
        assert_eq!(scratch.source(provenance)[0].ch, 'f');
        assert!(scratch.next_is_font_kern(0));
        assert_eq!(scratch.physical_nodes_through_boundary(0, 0, 1, 1), 2);
        assert!(
            matches!(scratch.output[1], Some(TfmCell::Kern { amount, .. }) if amount == Scaled::from_raw(17))
        );
    });
}
