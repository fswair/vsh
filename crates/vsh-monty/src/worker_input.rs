//! Supervised pipe writes without a writer-thread round trip on Unix.

use std::io;
#[cfg(unix)]
use std::io::Write;
use std::process::ChildStdin;
use std::thread;
use std::time::{Duration, Instant};

use monty_proto::{pb, write_frame};
use vsh_execution::ExecutionCancellation;

#[derive(Debug)]
pub(super) struct WorkerInput {
    #[cfg(unix)]
    stream: Option<ChildStdin>,
    #[cfg(not(unix))]
    writes: Option<std::sync::mpsc::SyncSender<WriteRequest>>,
    #[cfg(not(unix))]
    writer: Option<thread::JoinHandle<()>>,
}

#[cfg(not(unix))]
struct WriteRequest {
    request: pb::ParentRequest,
    completion: std::sync::mpsc::SyncSender<io::Result<()>>,
}

impl WorkerInput {
    pub(super) fn new(stream: ChildStdin) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
            let flags = fcntl_getfl(&stream)?;
            fcntl_setfl(&stream, flags | OFlags::NONBLOCK)?;
            Ok(Self {
                stream: Some(stream),
            })
        }
        #[cfg(not(unix))]
        {
            let (writes, requests) = std::sync::mpsc::sync_channel::<WriteRequest>(1);
            let writer = thread::Builder::new()
                .name("vsh-monty-worker-writer".into())
                .spawn(move || {
                    let mut stream = stream;
                    while let Ok(request) = requests.recv() {
                        let result = write_frame(&mut stream, &request.request).map_err(frame_io);
                        let failed = result.is_err();
                        let _ = request.completion.send(result);
                        if failed {
                            break;
                        }
                    }
                })?;
            Ok(Self {
                writes: Some(writes),
                writer: Some(writer),
            })
        }
    }

    #[cfg_attr(
        unix,
        expect(
            clippy::needless_pass_by_value,
            reason = "Windows queues an owned frame; Unix writes it synchronously"
        )
    )]
    pub(super) fn send(
        &mut self,
        request: pb::ParentRequest,
        deadline: Instant,
        cancellation: Option<&ExecutionCancellation>,
    ) -> io::Result<()> {
        check_control(deadline, cancellation)?;
        #[cfg(unix)]
        {
            write_frame(
                &mut ControlledWriter {
                    stream: self
                        .stream
                        .as_mut()
                        .ok_or_else(|| io::Error::other("writer unavailable"))?,
                    deadline,
                    cancellation,
                },
                &request,
            )
            .map_err(frame_io)?;
        }
        #[cfg(not(unix))]
        {
            let (completion, written) = std::sync::mpsc::sync_channel(1);
            self.writes
                .as_ref()
                .ok_or_else(|| io::Error::other("writer unavailable"))?
                .try_send(WriteRequest {
                    request,
                    completion,
                })
                .map_err(|_| io::Error::other("writer queue unavailable"))?;
            loop {
                check_control(deadline, cancellation)?;
                let remaining = deadline.saturating_duration_since(Instant::now());
                match written.recv_timeout(remaining.min(Duration::from_millis(20))) {
                    Ok(result) => {
                        result?;
                        break;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        return Err(io::Error::other("writer disconnected"));
                    }
                }
            }
        }
        check_control(deadline, cancellation)
    }

    // The owning Worker must kill/reap the child before joining a blocked writer.
    pub(super) fn close(&mut self) {
        #[cfg(unix)]
        self.stream.take();
        #[cfg(not(unix))]
        {
            self.writes.take();
            if let Some(writer) = self.writer.take() {
                let _ = writer.join();
            }
        }
    }
}

fn frame_io(error: monty_proto::FrameError) -> io::Error {
    match error {
        monty_proto::FrameError::Io(error) => error,
        error => io::Error::other(error),
    }
}

fn check_control(
    deadline: Instant,
    cancellation: Option<&ExecutionCancellation>,
) -> io::Result<()> {
    if cancellation.is_some_and(ExecutionCancellation::is_cancelled) {
        return Err(io::Error::other("worker write cancelled"));
    }
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "worker write deadline exceeded",
        ));
    }
    Ok(())
}

#[cfg(unix)]
struct ControlledWriter<'a> {
    stream: &'a mut ChildStdin,
    deadline: Instant,
    cancellation: Option<&'a ExecutionCancellation>,
}

#[cfg(unix)]
impl Write for ControlledWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            check_control(self.deadline, self.cancellation)?;
            match self.stream.write(bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        check_control(self.deadline, self.cancellation)?;
        self.stream.flush()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn blocked_pipe_honors_deadline_and_cancellation() {
        for cancel in [false, true] {
            let mut child = Command::new("python3")
                .args(["-c", "import time; time.sleep(10)"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let mut input = WorkerInput::new(child.stdin.take().unwrap()).unwrap();
            let token = ExecutionCancellation::default();
            let shared = token.clone();
            let canceller = thread::spawn(move || {
                if cancel {
                    thread::sleep(Duration::from_millis(20));
                    assert!(shared.cancel());
                }
            });
            let started = Instant::now();
            let result = input.send(
                pb::ParentRequest {
                    trace_parent: None,
                    kind: Some(pb::parent_request::Kind::Feed(pb::Feed {
                        code: "x".repeat(256 * 1024),
                        inputs: vec![],
                        skip_type_check: true,
                    })),
                },
                started + Duration::from_millis(100),
                Some(&token),
            );
            let _ = child.kill();
            child.wait().unwrap();
            input.close();
            canceller.join().unwrap();
            assert!(result.is_err());
            assert!(started.elapsed() < Duration::from_secs(2));
            assert_eq!(
                result.unwrap_err().kind(),
                if cancel {
                    io::ErrorKind::Other
                } else {
                    io::ErrorKind::TimedOut
                }
            );
        }
    }
}
