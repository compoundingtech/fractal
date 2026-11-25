use std::{path::{Path, PathBuf}, sync::LazyLock};

pub fn agent_binary() -> &'static Path {
    static BINARY: LazyLock<PathBuf> = LazyLock::new(|| {
        if std::env::var_os("NEXTEST").is_some() {
            // Nextest rewrites this runtime value for authenticated archive consumers;
            // the compile-time Cargo path still points at the producer's checkout.
            std::env::var_os("NEXTEST_BIN_EXE_fractal-light-node-agent")
                .or_else(|| std::env::var_os("NEXTEST_BIN_EXE_fractal_light_node_agent"))
                .map(PathBuf::from).expect("nextest must provide the remapped standalone agent binary")
        } else {
            assert!(std::env::var_os("CI_TEST_EXTRACTED").is_none(),
                "an archive consumer must resolve its nextest-provided binary, not the producer path");
            PathBuf::from(env!("CARGO_BIN_EXE_fractal-light-node-agent"))
        }
    });
    BINARY.as_path()
}
