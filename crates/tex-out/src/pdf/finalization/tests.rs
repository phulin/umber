use super::{
    PDF_MAX_OBJECT_ID, PdfAllocationInput, PdfFinalizationLimits, PdfReservedDocumentObjects,
};

fn allocation(next_object: u32) -> PdfAllocationInput {
    PdfAllocationInput {
        document: PdfReservedDocumentObjects {
            pages: 1,
            names: Some(2),
            catalog: 3,
            info: Some(4),
        },
        next_object,
    }
}

#[test]
fn allocation_replays_identically_from_the_detached_boundary() {
    let mut first = allocation(50).allocator(PDF_MAX_OBJECT_ID);
    let mut second = allocation(50).allocator(PDF_MAX_OBJECT_ID);
    let plan = [1, 3, 2, 17, 1];
    let first_ids = plan.map(|count| first.allocate_many(count).expect("allocation fits"));
    let second_ids = plan.map(|count| second.allocate_many(count).expect("allocation fits"));
    assert_eq!(first_ids, [50, 51, 54, 56, 73]);
    assert_eq!(first_ids, second_ids);
    assert_eq!(first.next_object(), 74);
}

#[test]
fn allocation_fails_atomically_at_the_configured_limit() {
    let mut allocator = allocation(19).allocator(20);
    let error = allocator
        .allocate_many(3)
        .expect_err("range crosses explicit maximum");
    assert_eq!(error.next, 19);
    assert_eq!(error.count, 3);
    assert_eq!(allocator.next_object(), 19);
}

#[test]
fn limits_preserve_the_legacy_finalizer_budgets() {
    assert_eq!(
        PdfFinalizationLimits::default(),
        PdfFinalizationLimits {
            max_object_id: 2_147_483_647,
            max_form_depth: 256,
            max_form_work: 1_000_000,
            max_virtual_font_recursion: tex_fonts::PDFTEX_VF_MAX_RECURSION,
            max_virtual_font_stack_depth: 100,
            max_virtual_font_packet_commands: 1_000_000,
            max_virtual_font_output_operations: 1_000_000,
            max_virtual_font_special_bytes: 8 * 1024 * 1024,
            max_imported_stream_bytes: 256 * 1024 * 1024,
        }
    );
}
