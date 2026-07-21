use std::{io, process::ExitCode};

use cakesplitter_core::CancellationToken;

fn main() -> ExitCode {
    let cancellation = CancellationToken::new();
    let signal_cancellation = cancellation.clone();
    if let Err(error) = ctrlc::set_handler(move || signal_cancellation.cancel()) {
        eprintln!("cakesplitter: [signal_handler] {error}");
        return ExitCode::FAILURE;
    }

    let arguments = std::env::args_os().collect::<Vec<_>>();
    let stdout = io::stdout();
    let stderr = io::stderr();
    let mut stdout = stdout.lock();
    let mut stderr = stderr.lock();
    ExitCode::from(cakesplitter_cli::run(
        arguments,
        &mut stdout,
        &mut stderr,
        cancellation,
    ))
}
