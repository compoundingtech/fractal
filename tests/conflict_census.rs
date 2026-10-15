use std::process::Command;

#[test]
fn conflict_census_finds_conflicts_on_a_throwaway_repository() {
    let output = Command::new("python3")
        .arg("scripts/conflict-census-test")
        .current_dir(test_env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run the conflict census test");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
