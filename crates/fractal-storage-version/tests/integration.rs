#[test]
fn reference_and_dependency_readers_precede_the_default_writer_format() {
    assert_eq!(
        fractal_storage_version::STORAGE_VERSION,
        20,
        "Reference21 and dependency22 readers ship before either default writer activation"
    );
    assert_eq!(fractal_storage_version::MAX_SUPPORTED_STORAGE_VERSION, 23);
}
