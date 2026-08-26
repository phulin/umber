#[test]
fn html_mvp_catalog_is_explicitly_unavailable_until_ahash64_republication() {
    let catalog: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tools/texlive-wasm-publish/catalog/html-mvp-v1.json"
    ))
    .expect("parse unavailable catalog marker");
    assert_eq!(catalog["schema"], 0);
    assert!(
        catalog["unavailable"]
            .as_str()
            .expect("unavailable reason")
            .contains("umber2-66p0.27")
    );
}
