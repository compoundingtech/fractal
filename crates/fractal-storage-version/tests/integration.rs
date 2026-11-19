#[test]
fn signature_reference_upgrade_requires_its_assigned_storage_version() {
    assert_eq!(
        fractal_storage_version::STORAGE_VERSION,
        20,
        "The reference reader must ship before the default format changes to21"
    );
    assert_eq!(fractal_storage_version::MAX_SUPPORTED_STORAGE_VERSION, 21);
}
