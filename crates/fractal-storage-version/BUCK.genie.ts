import { buckRootFile } from '../../buck2/root.ts'

// Reuse the consumer's pinned Prelude and Nix-capability native Rust toolchains.
// This slice deliberately has no Cargo dependency/build-script translation.
export default buckRootFile({
  text: `load("@prelude//:prelude.bzl", "native")

native.rust_library(
    name = "fractal-storage-version",
    crate = "fractal_storage_version",
    crate_root = "src/lib.rs",
    srcs = glob(["src/**/*.rs"]) + ["Cargo.toml"],
    edition = "2024",
    visibility = ["PUBLIC"],
)

native.rust_test(
    name = "unit-tests",
    crate = "fractal_storage_version",
    crate_root = "src/lib.rs",
    srcs = glob(["src/**/*.rs"]) + ["Cargo.toml"],
    edition = "2024",
)

native.rust_test(
    name = "integration-tests",
    crate = "integration",
    crate_root = "tests/integration.rs",
    srcs = glob(["tests/**/*.rs"]) + ["Cargo.toml"],
    edition = "2024",
    deps = [":fractal-storage-version"],
)
`,
})
