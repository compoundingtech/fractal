#[test]
fn thread_message_reader_upgrade_requires_its_assigned_storage_version() {
    assert_eq!(
        fractal_storage_version::STORAGE_VERSION,
        22,
        "If another incompatible upgrade is queued first, adopt its shared crate and advance this upgrade, its assertions, docs and release transition together"
    );
}
