//! CLI stdout writes share one exit policy: a reader closing its pipe is success.
//! Apply this only to output errors, never to daemon or filesystem errors.

use std::fmt;
use std::io::{self, Write as _};
use std::pin::Pin;
use std::process::ExitCode;
use std::task::{Context, Poll};

#[macro_export]
macro_rules! cli_print {
    ($($arg:tt)*) => { $crate::cli_output::print(format_args!($($arg)*)) };
}

#[macro_export]
macro_rules! cli_println {
    () => { $crate::cli_print!("\n") };
    ($($arg:tt)*) => { $crate::cli_print!("{}\n", format_args!($($arg)*)) };
}

// Keep the ordinary formatting syntax for every CLI command and its child modules.
macro_rules! print {
    ($($arg:tt)*) => { $crate::cli_print!($($arg)*) };
}

macro_rules! println {
    ($($arg:tt)*) => { $crate::cli_println!($($arg)*) };
}

pub fn print(arguments: fmt::Arguments<'_>) {
    if let Err(error) = io::stdout().lock().write_fmt(arguments) {
        let error = write_error(error);
        let _ = writeln!(io::stderr().lock(), "st: write stdout: {error}");
        std::process::exit(1);
    }
}

/// Streaming and raw byte output use the same policy as formatted output. Exit at the
/// failed write, so a follow command stops immediately and no later mutation is attempted.
fn write_error(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::BrokenPipe {
        std::process::exit(0);
    }
    error
}

/// Include a final unterminated line in the policy; std's shutdown flush ignores errors.
pub fn finish(code: ExitCode) -> ExitCode {
    match io::stdout().flush() {
        Ok(()) => code,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => code,
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "st: flush stdout: {error}");
            ExitCode::FAILURE
        }
    }
}

/// A stdout writer applying the CLI policy to raw and streaming output too.
pub struct Output<W>(W);

pub fn stdout() -> Output<io::Stdout> {
    Output(io::stdout())
}

pub fn async_stdout() -> Output<tokio::io::Stdout> {
    Output(tokio::io::stdout())
}

impl Output<io::Stdout> {
    pub fn is_terminal(&self) -> bool {
        use io::IsTerminal as _;
        self.0.is_terminal()
    }
}

impl<W: io::Write> io::Write for Output<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes).map_err(write_error)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush().map_err(write_error)
    }
}

impl<W: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for Output<W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0)
            .poll_write(context, bytes)
            .map_err(write_error)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0)
            .poll_flush(context)
            .map_err(write_error)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0)
            .poll_shutdown(context)
            .map_err(write_error)
    }
}
