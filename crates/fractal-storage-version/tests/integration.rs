#[test]
fn reader_first_default_is_separate_from_the_causal_thread_privacy_floor() {
    assert_eq!(
        fractal_storage_version::STORAGE_VERSION,
        20,
        "Reference21 and dependency22 readers precede default writer activation"
    );
    assert_eq!(fractal_storage_version::MAX_SUPPORTED_STORAGE_VERSION, 23);
    assert_eq!(fractal_storage_version::THREAD_PRIVACY_STORAGE_VERSION, 23);
    assert!(
        fractal_storage_version::THREAD_PRIVACY_STORAGE_VERSION > 22,
        "the privacy floor must be refused by the previous reader's max22 gate"
    );
}
