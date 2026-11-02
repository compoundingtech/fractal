// The build identity is baked here and handed to the libraries first thing, before anything reads the
// version: clap's `--version`, telemetry, the API, hook receipts, stui (see crates/fractal-build-stamp).
fn main() -> std::process::ExitCode {
    fractal::main(
        fractal_build_stamp::CLI_BUILD_STAMP,
        fractal_build_stamp::ST_BUILD_STAMP_LOCAL,
    )
}
