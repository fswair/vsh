use std::borrow::Cow;
use std::mem;
use std::time::Duration;

use monty::{MontyRun, RunProgress};
use monty_proto::{
    FrameError, WireFunctionCall, check_protocol_version, future_results_from_proto, pb,
};
use monty_types::{
    CompileOptions, ExcType, ExtFunctionResult, MontyException, NameLookupResult, PrintWriter,
    PrintWriterCallback, ResourceLimits, ResourceTracker,
};

pub(crate) trait Sink {
    fn send(&mut self, event: &pb::ChildEvent) -> Result<(), FrameError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HandleOutcome {
    Continue,
    Shutdown,
    Fatal,
}

#[derive(Default)]
pub(crate) struct ProtocolChild {
    state: SessionState,
}

#[derive(Default)]
enum SessionState {
    #[default]
    Empty,
    Configured(SessionConfig),
    Suspended(Box<RunProgress>),
    Finished,
}

struct SessionConfig {
    script_name: String,
    limits: ResourceLimits,
}

impl ProtocolChild {
    pub(crate) fn handle(
        &mut self,
        request: pb::ParentRequest,
        sink: &mut dyn Sink,
    ) -> Result<HandleOutcome, FrameError> {
        let Some(kind) = request.kind else {
            sink.send(&protocol_violation("request has no kind"))?;
            return Ok(HandleOutcome::Continue);
        };
        match kind {
            pb::parent_request::Kind::Configure(configure) => {
                return self.configure(configure, sink);
            }
            pb::parent_request::Kind::Feed(feed) => self.feed(feed, sink)?,
            pb::parent_request::Kind::ResumeCall(resume) => {
                self.resume_call(resume, sink)?;
            }
            pb::parent_request::Kind::ResumeNameLookup(resume) => {
                self.resume_name_lookup(resume, sink)?;
            }
            pb::parent_request::Kind::ResumeFutures(resume) => {
                self.resume_futures(resume, sink)?;
            }
            pb::parent_request::Kind::Reset(_) => self.reset(sink)?,
            pb::parent_request::Kind::Shutdown(_) => {
                sink.send(&event(pb::child_event::Kind::Shutdown(pb::ShutdownDump {
                    dump: None,
                })))?;
                return Ok(HandleOutcome::Shutdown);
            }
            pb::parent_request::Kind::InstallDependencies(_)
            | pb::parent_request::Kind::Dump(_)
            | pb::parent_request::Kind::Load(_) => {
                sink.send(&protocol_violation(
                    "request is unsupported by the VSH worker",
                ))?;
            }
        }
        Ok(HandleOutcome::Continue)
    }

    fn configure(
        &mut self,
        configure: pb::Configure,
        sink: &mut dyn Sink,
    ) -> Result<HandleOutcome, FrameError> {
        if let Err(message) = check_protocol_version(configure.protocol_version) {
            sink.send(&fatal_error_event(&message))?;
            return Ok(HandleOutcome::Fatal);
        }
        if !matches!(self.state, SessionState::Empty) {
            sink.send(&protocol_violation("Configure requires an empty worker"))?;
            return Ok(HandleOutcome::Continue);
        }
        if configure.type_check {
            sink.send(&protocol_violation(
                "type checking is disabled in the VSH worker",
            ))?;
            return Ok(HandleOutcome::Continue);
        }
        let limits = match decode_limits(configure.limits) {
            Ok(limits) => limits,
            Err(message) => {
                sink.send(&protocol_violation(message))?;
                return Ok(HandleOutcome::Continue);
            }
        };
        if let Err(message) = monty_alloc::set_limit(limits.max_memory, false) {
            sink.send(&fatal_error_event(message))?;
            return Ok(HandleOutcome::Fatal);
        }
        self.state = SessionState::Configured(SessionConfig {
            script_name: configure.script_name,
            limits,
        });
        sink.send(&event(pb::child_event::Kind::Ok(pb::Ok {})))?;
        Ok(HandleOutcome::Continue)
    }

    fn feed(&mut self, feed: pb::Feed, sink: &mut dyn Sink) -> Result<(), FrameError> {
        if !feed.inputs.is_empty() {
            sink.send(&protocol_violation("named Feed inputs are unsupported"))?;
            return Ok(());
        }
        let SessionState::Configured(config) = mem::take(&mut self.state) else {
            sink.send(&protocol_violation("Feed requires a configured worker"))?;
            return Ok(());
        };
        let run = match MontyRun::new(
            feed.code,
            &config.script_name,
            Vec::new(),
            CompileOptions::default(),
        ) {
            Ok(run) => run,
            Err(error) => {
                self.state = SessionState::Finished;
                return sink.send(&error_event(&error));
            }
        };
        let tracker = ResourceTracker::new(config.limits);
        let progress = with_print(sink, |print| run.start(Vec::new(), tracker, print));
        self.accept_progress(progress, sink)
    }

    fn resume_call(
        &mut self,
        resume: pb::ResumeCall,
        sink: &mut dyn Sink,
    ) -> Result<(), FrameError> {
        let call_id = resume.call_id;
        let result = resume
            .result
            .and_then(|result| ExtFunctionResult::try_from(result).ok());
        let Some(result) = result else {
            sink.send(&protocol_violation("ResumeCall result is invalid"))?;
            return Ok(());
        };
        let SessionState::Suspended(progress) = mem::take(&mut self.state) else {
            sink.send(&protocol_violation(
                "ResumeCall requires a suspended worker",
            ))?;
            return Ok(());
        };
        let expected = match progress.as_ref() {
            RunProgress::OsCall(call) => call.call_id,
            RunProgress::FunctionCall(call) => call.call_id,
            _ => {
                self.state = SessionState::Suspended(progress);
                sink.send(&protocol_violation("suspension does not accept ResumeCall"))?;
                return Ok(());
            }
        };
        if call_id != expected {
            self.state = SessionState::Suspended(progress);
            sink.send(&protocol_violation("ResumeCall call_id does not match"))?;
            return Ok(());
        }
        let next = with_print(sink, |print| match *progress {
            RunProgress::OsCall(call) => call.resume(result, print),
            RunProgress::FunctionCall(call) => call.resume(result, print),
            _ => unreachable!("validated call suspension"),
        });
        self.accept_progress(next, sink)
    }

    fn resume_name_lookup(
        &mut self,
        resume: pb::ResumeNameLookup,
        sink: &mut dyn Sink,
    ) -> Result<(), FrameError> {
        let result = match resume.kind {
            Some(pb::resume_name_lookup::Kind::Value(value)) => {
                let Ok(value) = value.into_object() else {
                    sink.send(&protocol_violation("name lookup value is invalid"))?;
                    return Ok(());
                };
                NameLookupResult::Value(value)
            }
            Some(pb::resume_name_lookup::Kind::Undefined(_)) => NameLookupResult::Undefined,
            None => {
                sink.send(&protocol_violation("name lookup result has no kind"))?;
                return Ok(());
            }
        };
        let SessionState::Suspended(progress) = mem::take(&mut self.state) else {
            sink.send(&protocol_violation(
                "ResumeNameLookup requires a suspended worker",
            ))?;
            return Ok(());
        };
        let RunProgress::NameLookup(lookup) = *progress else {
            self.state = SessionState::Suspended(progress);
            sink.send(&protocol_violation(
                "suspension does not accept ResumeNameLookup",
            ))?;
            return Ok(());
        };
        let next = with_print(sink, |print| lookup.resume(result, print));
        self.accept_progress(next, sink)
    }

    fn resume_futures(
        &mut self,
        resume: pb::ResumeFutures,
        sink: &mut dyn Sink,
    ) -> Result<(), FrameError> {
        let results = match future_results_from_proto(resume.results) {
            Ok(results) => results,
            Err(error) => {
                sink.send(&protocol_violation(&error.to_string()))?;
                return Ok(());
            }
        };
        let SessionState::Suspended(progress) = mem::take(&mut self.state) else {
            sink.send(&protocol_violation(
                "ResumeFutures requires a suspended worker",
            ))?;
            return Ok(());
        };
        let RunProgress::ResolveFutures(futures) = *progress else {
            self.state = SessionState::Suspended(progress);
            sink.send(&protocol_violation(
                "suspension does not accept ResumeFutures",
            ))?;
            return Ok(());
        };
        let next = with_print(sink, |print| futures.resume(results, print));
        self.accept_progress(next, sink)
    }

    fn reset(&mut self, sink: &mut dyn Sink) -> Result<(), FrameError> {
        self.state = SessionState::Empty;
        if let Err(message) = monty_alloc::set_limit(None, false) {
            sink.send(&fatal_error_event(message))
        } else {
            sink.send(&event(pb::child_event::Kind::Ok(pb::Ok {})))
        }
    }

    fn accept_progress(
        &mut self,
        progress: Result<RunProgress, MontyException>,
        sink: &mut dyn Sink,
    ) -> Result<(), FrameError> {
        let progress = match progress {
            Ok(progress) => progress,
            Err(error) => {
                self.state = SessionState::Finished;
                return sink.send(&error_event(&error));
            }
        };
        match progress {
            RunProgress::Complete(value) => {
                self.state = SessionState::Finished;
                sink.send(&event(pb::child_event::Kind::Complete(pb::Complete {
                    value: Some(value.into()),
                })))
            }
            RunProgress::OsCall(call) => {
                let event = event(pb::child_event::Kind::OsCall(pb::OsCall {
                    call_id: call.call_id,
                    call: Some(call.function_call.clone().into()),
                }));
                self.state = SessionState::Suspended(Box::new(RunProgress::OsCall(call)));
                sink.send(&event)
            }
            RunProgress::FunctionCall(call) => {
                let event = event(pb::child_event::Kind::FunctionCall(WireFunctionCall {
                    function_name: call.function_name.clone(),
                    args: call.args.clone(),
                    kwargs: call.kwargs.clone(),
                    call_id: call.call_id,
                    method_call: call.method_call,
                }));
                self.state = SessionState::Suspended(Box::new(RunProgress::FunctionCall(call)));
                sink.send(&event)
            }
            RunProgress::NameLookup(lookup) => {
                let event = event(pb::child_event::Kind::NameLookup(pb::NameLookup {
                    name: lookup.name.clone(),
                }));
                self.state = SessionState::Suspended(Box::new(RunProgress::NameLookup(lookup)));
                sink.send(&event)
            }
            RunProgress::ResolveFutures(futures) => {
                let event = event(pb::child_event::Kind::ResolveFutures(pb::ResolveFutures {
                    pending_call_ids: futures.pending_call_ids().to_vec(),
                }));
                self.state =
                    SessionState::Suspended(Box::new(RunProgress::ResolveFutures(futures)));
                sink.send(&event)
            }
        }
    }
}

fn decode_limits(encoded: Option<pb::ResourceLimits>) -> Result<ResourceLimits, &'static str> {
    let Some(encoded) = encoded else {
        return Ok(ResourceLimits::default());
    };
    let max_memory = encoded
        .max_memory_bytes
        .map(usize::try_from)
        .transpose()
        .map_err(|_| "memory limit does not fit this worker")?;
    let gc_interval = encoded
        .gc_interval
        .map(usize::try_from)
        .transpose()
        .map_err(|_| "GC interval does not fit this worker")?;
    let max_recursion_depth = encoded
        .max_recursion_depth
        .map(usize::try_from)
        .transpose()
        .map_err(|_| "recursion limit does not fit this worker")?
        .unwrap_or(monty_types::DEFAULT_MAX_RECURSION_DEPTH);
    Ok(ResourceLimits {
        max_duration: encoded.max_duration_micros.map(Duration::from_micros),
        max_memory,
        gc_interval,
        max_recursion_depth,
    })
}

fn with_print(
    sink: &mut dyn Sink,
    operation: impl FnOnce(PrintWriter<'_>) -> Result<RunProgress, MontyException>,
) -> Result<RunProgress, MontyException> {
    let mut output = ProtoPrint::new(sink);
    let result = operation(PrintWriter::Callback(&mut output));
    output.drain();
    result
}

struct ProtoPrint<'a> {
    buffer: String,
    sink: &'a mut dyn Sink,
}

impl<'a> ProtoPrint<'a> {
    const FLUSH_BYTES: usize = 8 * 1024;

    fn new(sink: &'a mut dyn Sink) -> Self {
        Self {
            buffer: String::new(),
            sink,
        }
    }

    fn flush(&mut self) -> Result<(), MontyException> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        self.sink
            .send(&event(pb::child_event::Kind::Print(pb::Print {
                stream: pb::PrintStream::Stdout.into(),
                text: mem::take(&mut self.buffer),
            })))
            .map_err(|error| {
                MontyException::new(
                    ExcType::RuntimeError,
                    Some(format!("failed to stream print output: {error}")),
                )
            })
    }

    fn maybe_flush(&mut self) -> Result<(), MontyException> {
        if self.buffer.ends_with('\n') || self.buffer.len() >= Self::FLUSH_BYTES {
            self.flush()
        } else {
            Ok(())
        }
    }

    fn drain(&mut self) {
        let _ = self.flush();
    }
}

impl PrintWriterCallback for ProtoPrint<'_> {
    fn stdout_write(&mut self, output: Cow<'_, str>) -> Result<(), MontyException> {
        let mut remaining = output.as_ref();
        while !remaining.is_empty() {
            let take = floor_char_boundary(remaining, Self::FLUSH_BYTES - self.buffer.len());
            if take == 0 {
                self.flush()?;
                continue;
            }
            self.buffer.push_str(&remaining[..take]);
            remaining = &remaining[take..];
            self.maybe_flush()?;
        }
        Ok(())
    }

    fn stdout_push(&mut self, end: char) -> Result<(), MontyException> {
        self.buffer.push(end);
        self.maybe_flush()
    }
}

fn floor_char_boundary(value: &str, maximum: usize) -> usize {
    if maximum >= value.len() {
        return value.len();
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn event(kind: pb::child_event::Kind) -> pb::ChildEvent {
    pb::ChildEvent {
        total_execution_micros: 0,
        max_duration_micros: None,
        restored_script_name: None,
        kind: Some(kind),
    }
}

fn error_event(error: &MontyException) -> pb::ChildEvent {
    event(pb::child_event::Kind::Error(pb::Error {
        exception: Some(error.into()),
    }))
}

pub(crate) fn protocol_violation(message: &str) -> pb::ChildEvent {
    event(pb::child_event::Kind::Error(pb::Error {
        exception: Some(pb::RaisedException {
            exc_type: ExcType::RuntimeError.to_string(),
            message: Some(format!("protocol violation: {message}")),
            traceback: Vec::new(),
            data: None,
        }),
    }))
}

pub(crate) fn fatal_error_event(message: &str) -> pb::ChildEvent {
    event(pb::child_event::Kind::FatalError(pb::FatalError {
        message: message.to_owned(),
    }))
}
