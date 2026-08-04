use super::*;

#[test]
fn pre_staging_shipout_error_uses_live_command_context() {
    // TeX82 §§82 and 641: `ship_out` reports the huge-page error against the
    // current input stack. Successful artifact staging republishes its
    // detached summary only later, so the command-owned entry context wins.
    let stores = Universe::new();
    let stale = tex_state::InputSummary::default();
    let origin = ShipoutOrigin {
        output_open_context: Some("\n<recently read> }\n                  ".to_owned()),
        pending_end: 0,
        announce_openout: false,
    };

    assert_eq!(
        shipout_error_context(&stores, &stale, &origin),
        "\n<recently read> }\n                  "
    );
}
