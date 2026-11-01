// The production CLI for this package's tests, with no build identity: the same entry point as the installed
// fractal (crates/fractal-cli), which differs only by the identity it bakes. Unlike the fixture it never sets
// fixture mode. Only test builds link it (see Cargo.toml).
fn main() -> std::process::ExitCode {
    fractal::main(None, None)
}
