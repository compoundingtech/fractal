// A separate, non-installed test executable runs the same CLI as fractal. Only it calls `fixture_main`, which
// exists only with test-support: renaming fractal or setting an environment variable cannot enable it. It
// bakes no build identity, so its version is the package's with an unknown revision.
fn main() -> std::process::ExitCode {
    fractal::fixture_main()
}
