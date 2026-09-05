//! Minimal stdio shell around Monty's public typed execution seam.

use std::io;
use std::panic;
use std::process::ExitCode;

use monty_proto::{FrameError, FrameReader, pb, write_frame};

use crate::child::{HandleOutcome, ProtocolChild, Sink, fatal_error_event, protocol_violation};

mod child;

const MONTY_VERSION: &str = "0.0.22";
const EX_PROTOCOL: u8 = 76;

#[global_allocator]
static ALLOCATOR: monty_alloc::LimitedAllocator = monty_alloc::LimitedAllocator;

fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    match (arguments.next(), arguments.next()) {
        (Some(argument), None) if argument == "--version" => {
            println!("vsh-monty-worker {MONTY_VERSION}");
            ExitCode::SUCCESS
        }
        (Some(argument), None) if argument == "subprocess" => run_worker(),
        _ => {
            eprintln!("usage: vsh-monty-worker subprocess | --version");
            ExitCode::from(2)
        }
    }
}

fn run_worker() -> ExitCode {
    install_panic_hook();
    let mut reader = FrameReader::new(io::stdin().lock());
    let mut child = ProtocolChild::default();
    let mut sink = StdoutSink;

    loop {
        match reader.read::<pb::ParentRequest>() {
            Ok(Some(request)) => match child.handle(request, &mut sink) {
                Ok(HandleOutcome::Continue) => {}
                Ok(HandleOutcome::Shutdown) => return ExitCode::SUCCESS,
                Ok(HandleOutcome::Fatal) => return ExitCode::from(4),
                Err(FrameError::FrameTooLarge { len, max }) => {
                    fatal(
                        &mut sink,
                        &format!("response frame of {len} bytes exceeds maximum of {max} bytes"),
                    );
                    return ExitCode::from(EX_PROTOCOL);
                }
                Err(_) => return ExitCode::from(3),
            },
            Ok(None) => return ExitCode::SUCCESS,
            Err(FrameError::Decode(error)) => {
                if sink
                    .send(&protocol_violation(&format!("malformed request: {error}")))
                    .is_err()
                {
                    return ExitCode::from(3);
                }
            }
            Err(error) => {
                fatal(&mut sink, &format!("malformed request frame: {error}"));
                return ExitCode::from(EX_PROTOCOL);
            }
        }
    }
}

struct StdoutSink;

impl Sink for StdoutSink {
    fn send(&mut self, event: &pb::ChildEvent) -> Result<(), FrameError> {
        write_frame(&mut io::stdout(), event)
    }
}

fn fatal(sink: &mut impl Sink, message: &str) {
    eprintln!("vsh Monty worker fatal error: {message}");
    let _ = sink.send(&fatal_error_event(message));
}

fn install_panic_hook() {
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |information| {
        let _ = write_frame(
            &mut io::stdout(),
            &fatal_error_event(&format!("child panicked: {information}")),
        );
        default_hook(information);
    }));
}
