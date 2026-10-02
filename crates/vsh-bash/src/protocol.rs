//! Private bounded wire format, independent of Monty's protobuf protocol.

use std::io::{self, Read, Write};

pub(crate) const WORKER_ID: &str = concat!(
    "vsh-bash-worker/4 bashkit/0.18.2 vsh/",
    env!("CARGO_PKG_VERSION")
);
pub(crate) const PROFILE: &str = "vsh-bash-bounded-v4";
pub(crate) const HARD_FRAME_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_DIAGNOSTIC_BYTES: usize = 4096;
const MAX_PATH_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct DecodeLimits {
    pub(crate) frame_bytes: usize,
    pub(crate) io_bytes: usize,
    pub(crate) path_bytes: usize,
    pub(crate) output_bytes: usize,
    pub(crate) messages: u8,
}

impl DecodeLimits {
    pub(crate) const fn all(frame_bytes: usize) -> Self {
        Self {
            frame_bytes,
            io_bytes: HARD_FRAME_BYTES,
            path_bytes: MAX_PATH_BYTES,
            output_bytes: HARD_FRAME_BYTES,
            messages: u8::MAX,
        }
    }
}

/// Guest interpreter work ceilings, shared by nested execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BashLimits {
    /// Total interpreter/builtin work units.
    pub max_work_units: u64,
    /// Aggregate intermediate input, not just host filesystem reads.
    pub max_aggregate_input_bytes: u64,
    /// Simultaneously retained intermediate byte buffers.
    pub max_live_intermediate_bytes: u64,
    /// Maximum executed commands.
    pub max_commands: usize,
    /// Maximum iterations of a single loop.
    pub max_loop_iterations: usize,
    /// Maximum iterations across all loops.
    pub max_total_loop_iterations: usize,
    /// Maximum parser operations, including nested scripts.
    pub max_parser_operations: usize,
}

impl Default for BashLimits {
    fn default() -> Self {
        Self {
            max_work_units: 10_000_000,
            max_aggregate_input_bytes: 100_000_000,
            max_live_intermediate_bytes: 16 * 1024 * 1024,
            max_commands: 10_000,
            max_loop_iterations: 10_000,
            max_total_loop_iterations: 1_000_000,
            max_parser_operations: 100_000,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Run {
    pub(crate) code: String,
    pub(crate) limits: BashLimits,
    pub(crate) duration_us: u64,
    pub(crate) max_program_bytes: usize,
    pub(crate) max_memory_bytes: usize,
    pub(crate) max_output_bytes: usize,
    pub(crate) max_recursion_depth: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Metadata {
    pub(crate) kind: u8,
    pub(crate) size: u64,
    pub(crate) mode: u32,
}

#[derive(Debug)]
pub(crate) enum FsRequest {
    Read(String),
    Write(String, Vec<u8>),
    Append(String, Vec<u8>),
    Mkdir(String, bool),
    Remove(String, bool),
    Stat(String),
    ReadDir(String),
    Exists(String),
    Rename(String, String),
    Copy(String, String),
    ReadLink(String),
    Chmod(String, u32),
    Unsupported(String, String),
}

#[derive(Debug)]
pub(crate) enum FsValue {
    Unit,
    Bytes(Vec<u8>),
    Metadata(Metadata),
    Entries(Vec<(String, Metadata)>),
    Bool(bool),
}

#[derive(Debug)]
pub(crate) struct FsFault {
    pub(crate) kind: u8,
    pub(crate) detail: String,
}

#[derive(Debug)]
pub(crate) enum Message {
    Hello(String),
    Run(Run),
    Call(FsRequest),
    Reply(Result<FsValue, FsFault>),
    Done {
        exit_code: i32,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        failure: Option<String>,
    },
    Ready,
    Shutdown,
}

#[derive(Debug)]
pub(crate) struct Frame {
    pub(crate) session: u64,
    pub(crate) sequence: u64,
    pub(crate) message: Message,
}

impl Frame {
    pub(crate) fn encode(&self, maximum: usize) -> io::Result<Vec<u8>> {
        let mut out = Encoder {
            bytes: Vec::new(),
            maximum: maximum.min(HARD_FRAME_BYTES),
        };
        let tag = match &self.message {
            Message::Hello(_) => 1,
            Message::Run(_) => 2,
            Message::Call(_) => 3,
            Message::Reply(_) => 4,
            Message::Done { .. } => 5,
            Message::Ready => 6,
            Message::Shutdown => 7,
        };
        out.byte(tag)?;
        out.u64(self.session)?;
        out.u64(self.sequence)?;
        match &self.message {
            Message::Hello(value) => out.string(value)?,
            Message::Run(run) => {
                encode_limits(run.limits, &mut out)?;
                for value in [
                    run.duration_us,
                    run.max_program_bytes as u64,
                    run.max_memory_bytes as u64,
                    run.max_output_bytes as u64,
                    run.max_recursion_depth as u64,
                ] {
                    out.u64(value)?;
                }
                out.string(&run.code)?;
            }
            Message::Call(call) => encode_call(call, &mut out)?,
            Message::Reply(result) => match result {
                Ok(value) => {
                    out.byte(0)?;
                    encode_value(value, &mut out)?;
                }
                Err(error) => {
                    out.byte(error.kind)?;
                    out.string(&error.detail)?;
                }
            },
            Message::Done {
                exit_code,
                stdout,
                stderr,
                failure,
            } => {
                out.extend(&exit_code.to_le_bytes())?;
                out.bytes(stdout)?;
                out.bytes(stderr)?;
                out.byte(u8::from(failure.is_some()))?;
                if let Some(failure) = failure {
                    out.string(failure)?;
                }
            }
            Message::Ready | Message::Shutdown => {}
        }
        Ok(out.bytes)
    }

    #[cfg(test)]
    pub(crate) fn decode(bytes: &[u8]) -> io::Result<Self> {
        Self::decode_bounded(bytes, DecodeLimits::all(HARD_FRAME_BYTES))
    }

    fn decode_bounded(bytes: &[u8], limits: DecodeLimits) -> io::Result<Self> {
        let mut input = Decoder { bytes, offset: 0 };
        let tag = input.byte()?;
        if tag > 7 || limits.messages & (1 << tag) == 0 {
            return Err(invalid("message direction is not permitted"));
        }
        let session = input.u64()?;
        let sequence = input.u64()?;
        let message = match tag {
            1 => Message::Hello(input.string(MAX_DIAGNOSTIC_BYTES)?),
            2 => {
                let limits = decode_limits(&mut input)?;
                let duration_us = input.u64()?;
                let max_program_bytes = input.usize()?;
                let max_memory_bytes = input.usize()?;
                let max_output_bytes = input.usize()?;
                let max_recursion_depth = input.usize()?;
                if max_program_bytes > HARD_FRAME_BYTES
                    || max_output_bytes > HARD_FRAME_BYTES
                    || max_memory_bytes == 0
                {
                    return Err(invalid("invalid run limits"));
                }
                Message::Run(Run {
                    code: input.string(max_program_bytes)?,
                    limits,
                    duration_us,
                    max_program_bytes,
                    max_memory_bytes,
                    max_output_bytes,
                    max_recursion_depth,
                })
            }
            3 => Message::Call(decode_call(&mut input, limits)?),
            4 => {
                let kind = input.byte()?;
                Message::Reply(if kind == 0 {
                    Ok(decode_value(&mut input)?)
                } else if kind <= 8 {
                    Err(FsFault {
                        kind,
                        detail: input.string(MAX_DIAGNOSTIC_BYTES)?,
                    })
                } else {
                    return Err(invalid("invalid filesystem fault kind"));
                })
            }
            5 => {
                let exit_code =
                    i32::from_le_bytes(input.take(4)?.try_into().expect("bounded exit code"));
                let stdout = input.bytes(limits.output_bytes)?.to_vec();
                let stderr = input
                    .bytes(limits.output_bytes.saturating_sub(stdout.len()))?
                    .to_vec();
                let failure = if input.boolean()? {
                    Some(input.string(MAX_DIAGNOSTIC_BYTES)?)
                } else {
                    None
                };
                Message::Done {
                    exit_code,
                    stdout,
                    stderr,
                    failure,
                }
            }
            6 => Message::Ready,
            7 => Message::Shutdown,
            _ => return Err(invalid("unknown frame kind")),
        };
        if input.offset != bytes.len() {
            return Err(invalid("trailing frame data"));
        }
        Ok(Self {
            session,
            sequence,
            message,
        })
    }
}

#[cfg(any(feature = "worker", test))]
pub(crate) fn read_frame(reader: &mut impl Read, maximum: usize) -> io::Result<Frame> {
    read_frame_with_cap(reader, || maximum)
}

#[cfg(any(feature = "worker", test))]
pub(crate) fn read_frame_with_cap(
    reader: &mut impl Read,
    maximum: impl FnOnce() -> usize,
) -> io::Result<Frame> {
    read_frame_with_limits(reader, || DecodeLimits::all(maximum()))
}

pub(crate) fn read_frame_with_limits(
    reader: &mut impl Read,
    negotiated: impl FnOnce() -> DecodeLimits,
) -> io::Result<Frame> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_le_bytes(header) as usize;
    let limits = negotiated();
    if length < 17 || length > limits.frame_bytes.min(HARD_FRAME_BYTES) {
        return Err(invalid("frame length exceeds negotiated boundary"));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| invalid("frame allocation failed"))?;
    bytes.resize(length, 0);
    reader.read_exact(&mut bytes)?;
    Frame::decode_bounded(&bytes, limits)
}

pub(crate) fn write_frame(
    writer: &mut impl Write,
    frame: &Frame,
    maximum: usize,
) -> io::Result<()> {
    let bytes = frame.encode(maximum)?;
    let length =
        u32::try_from(bytes.len()).map_err(|_| invalid("frame cannot fit length prefix"))?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

pub(crate) fn encode_limits(limits: BashLimits, out: &mut Encoder) -> io::Result<()> {
    for value in [
        limits.max_work_units,
        limits.max_aggregate_input_bytes,
        limits.max_live_intermediate_bytes,
        limits.max_commands as u64,
        limits.max_loop_iterations as u64,
        limits.max_total_loop_iterations as u64,
        limits.max_parser_operations as u64,
    ] {
        out.u64(value)?;
    }
    Ok(())
}

fn decode_limits(input: &mut Decoder<'_>) -> io::Result<BashLimits> {
    Ok(BashLimits {
        max_work_units: input.u64()?,
        max_aggregate_input_bytes: input.u64()?,
        max_live_intermediate_bytes: input.u64()?,
        max_commands: input.usize()?,
        max_loop_iterations: input.usize()?,
        max_total_loop_iterations: input.usize()?,
        max_parser_operations: input.usize()?,
    })
}

fn encode_call(call: &FsRequest, out: &mut Encoder) -> io::Result<()> {
    let (tag, path) = match call {
        FsRequest::Read(p) => (1, p),
        FsRequest::Write(p, _) => (2, p),
        FsRequest::Append(p, _) => (3, p),
        FsRequest::Mkdir(p, _) => (4, p),
        FsRequest::Remove(p, _) => (5, p),
        FsRequest::Stat(p) => (6, p),
        FsRequest::ReadDir(p) => (7, p),
        FsRequest::Exists(p) => (8, p),
        FsRequest::Rename(p, _) => (9, p),
        FsRequest::Copy(p, _) => (10, p),
        FsRequest::ReadLink(p) => (11, p),
        FsRequest::Chmod(p, _) => (12, p),
        FsRequest::Unsupported(p, _) => (13, p),
    };
    out.byte(tag)?;
    out.string(path)?;
    match call {
        FsRequest::Write(_, bytes) | FsRequest::Append(_, bytes) => out.bytes(bytes)?,
        FsRequest::Mkdir(_, flag) | FsRequest::Remove(_, flag) => out.byte(u8::from(*flag))?,
        FsRequest::Rename(_, target)
        | FsRequest::Copy(_, target)
        | FsRequest::Unsupported(_, target) => out.string(target)?,
        FsRequest::Chmod(_, mode) => out.extend(&mode.to_le_bytes())?,
        _ => {}
    }
    Ok(())
}

fn decode_call(input: &mut Decoder<'_>, limits: DecodeLimits) -> io::Result<FsRequest> {
    let tag = input.byte()?;
    if !(1..=13).contains(&tag) {
        return Err(invalid("invalid filesystem operation"));
    }
    let path = input.string(limits.path_bytes.min(MAX_PATH_BYTES))?;
    Ok(match tag {
        1 => FsRequest::Read(path),
        2 => FsRequest::Write(path, input.bytes(limits.io_bytes)?.to_vec()),
        3 => FsRequest::Append(path, input.bytes(limits.io_bytes)?.to_vec()),
        4 => FsRequest::Mkdir(path, input.boolean()?),
        5 => FsRequest::Remove(path, input.boolean()?),
        6 => FsRequest::Stat(path),
        7 => FsRequest::ReadDir(path),
        8 => FsRequest::Exists(path),
        9 => FsRequest::Rename(path, input.string(limits.path_bytes.min(MAX_PATH_BYTES))?),
        10 => FsRequest::Copy(path, input.string(limits.path_bytes.min(MAX_PATH_BYTES))?),
        11 => FsRequest::ReadLink(path),
        12 => FsRequest::Chmod(
            path,
            u32::from_le_bytes(input.take(4)?.try_into().expect("bounded mode")),
        ),
        13 => FsRequest::Unsupported(path, input.string(MAX_DIAGNOSTIC_BYTES)?),
        _ => return Err(invalid("invalid filesystem operation")),
    })
}

fn encode_metadata(state: Metadata, out: &mut Encoder) -> io::Result<()> {
    out.byte(state.kind)?;
    out.u64(state.size)?;
    out.extend(&state.mode.to_le_bytes())
}

fn decode_metadata(input: &mut Decoder<'_>) -> io::Result<Metadata> {
    let kind = input.byte()?;
    if !(1..=3).contains(&kind) {
        return Err(invalid("invalid node kind"));
    }
    Ok(Metadata {
        kind,
        size: input.u64()?,
        mode: u32::from_le_bytes(input.take(4)?.try_into().expect("bounded mode")),
    })
}

fn encode_value(value: &FsValue, out: &mut Encoder) -> io::Result<()> {
    match value {
        FsValue::Unit => out.byte(0)?,
        FsValue::Bytes(bytes) => {
            out.byte(1)?;
            out.bytes(bytes)?;
        }
        FsValue::Metadata(state) => {
            out.byte(2)?;
            encode_metadata(*state, out)?;
        }
        FsValue::Entries(entries) => {
            out.byte(3)?;
            out.u64(entries.len() as u64)?;
            for (name, state) in entries {
                out.string(name)?;
                encode_metadata(*state, out)?;
            }
        }
        FsValue::Bool(value) => {
            out.byte(4)?;
            out.byte(u8::from(*value))?;
        }
    }
    Ok(())
}

fn decode_value(input: &mut Decoder<'_>) -> io::Result<FsValue> {
    Ok(match input.byte()? {
        0 => FsValue::Unit,
        1 => FsValue::Bytes(input.bytes(HARD_FRAME_BYTES)?.to_vec()),
        2 => FsValue::Metadata(decode_metadata(input)?),
        3 => {
            let count = input.usize()?;
            // Every entry needs an eight-byte name length and thirteen-byte state.
            if count > (input.bytes.len() - input.offset) / 21 {
                return Err(invalid("directory count exceeds frame contents"));
            }
            let mut entries = Vec::new();
            entries
                .try_reserve_exact(count)
                .map_err(|_| invalid("directory allocation failed"))?;
            for _ in 0..count {
                entries.push((input.string(MAX_PATH_BYTES)?, decode_metadata(input)?));
            }
            FsValue::Entries(entries)
        }
        4 => FsValue::Bool(input.boolean()?),
        _ => return Err(invalid("invalid filesystem result")),
    })
}

pub(crate) struct Encoder {
    pub(crate) bytes: Vec<u8>,
    pub(crate) maximum: usize,
}

impl Encoder {
    fn extend(&mut self, bytes: &[u8]) -> io::Result<()> {
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| invalid("encoded frame length overflow"))?;
        if length > self.maximum {
            return Err(invalid("encoded frame exceeds boundary"));
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| invalid("encoded frame allocation failed"))?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn byte(&mut self, value: u8) -> io::Result<()> {
        self.extend(&[value])
    }
    fn u64(&mut self, value: u64) -> io::Result<()> {
        self.extend(&value.to_le_bytes())
    }
    fn bytes(&mut self, value: &[u8]) -> io::Result<()> {
        self.u64(value.len() as u64)?;
        self.extend(value)
    }
    fn string(&mut self, value: &str) -> io::Result<()> {
        self.bytes(value.as_bytes())
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Decoder<'a> {
    fn take(&mut self, length: usize) -> io::Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| invalid("decoded length overflow"))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| invalid("truncated frame contents"))?;
        self.offset = end;
        Ok(bytes)
    }
    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn boolean(&mut self) -> io::Result<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("invalid boolean")),
        }
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("bounded integer"),
        ))
    }
    fn usize(&mut self) -> io::Result<usize> {
        usize::try_from(self.u64()?).map_err(|_| invalid("host integer overflow"))
    }
    fn bytes(&mut self, maximum: usize) -> io::Result<&'a [u8]> {
        let length = self.usize()?;
        if length > maximum {
            return Err(invalid("nested byte limit exceeded"));
        }
        self.take(length)
    }
    fn string(&mut self, maximum: usize) -> io::Result<String> {
        let bytes = self.bytes(maximum)?;
        let value = std::str::from_utf8(bytes).map_err(|_| invalid("invalid UTF-8 string"))?;
        if value.contains('\0') {
            return Err(invalid("NUL in protocol string"));
        }
        Ok(value.to_owned())
    }
}

pub(crate) fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_preserve_binary_bytes_and_reject_trailing_or_oversized_data() {
        let frame = Frame {
            session: 3,
            sequence: 9,
            message: Message::Done {
                exit_code: 0,
                stdout: vec![0xff, 0, 0xfe],
                stderr: vec![0xfe],
                failure: None,
            },
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &frame, 256).unwrap();
        let decoded = read_frame(&mut bytes.as_slice(), 256).unwrap();
        let Message::Done { stdout, stderr, .. } = decoded.message else {
            panic!("wrong kind")
        };
        assert_eq!(stdout, [0xff, 0, 0xfe]);
        assert_eq!(stderr, [0xfe]);
        assert!(read_frame(&mut bytes.as_slice(), 20).is_err());
        assert!(Frame::decode(&[0; 17]).is_err());
        assert!(Frame::decode(&[6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]).is_err());
        assert!(read_frame(&mut bytes[..bytes.len() - 1].as_ref(), 256).is_err());
    }

    #[test]
    fn nested_lengths_counts_flags_and_strings_are_checked_before_allocation() {
        let mut out = Encoder {
            bytes: Vec::new(),
            maximum: 128,
        };
        out.byte(3).unwrap();
        out.u64(u64::MAX).unwrap();
        assert!(
            decode_value(&mut Decoder {
                bytes: &out.bytes,
                offset: 0
            })
            .is_err()
        );
        assert!(
            Decoder {
                bytes: &[2],
                offset: 0
            }
            .boolean()
            .is_err()
        );
        let mut out = Encoder {
            bytes: Vec::new(),
            maximum: 128,
        };
        out.u64(u64::MAX).unwrap();
        assert!(
            Decoder {
                bytes: &out.bytes,
                offset: 0
            }
            .string(128)
            .is_err()
        );
        let mut out = Encoder {
            bytes: Vec::new(),
            maximum: 128,
        };
        out.bytes(&[0xff]).unwrap();
        assert!(
            Decoder {
                bytes: &out.bytes,
                offset: 0
            }
            .string(128)
            .is_err()
        );
        let mut out = Encoder {
            bytes: Vec::new(),
            maximum: 128,
        };
        out.bytes(b"a\0b").unwrap();
        assert!(
            Decoder {
                bytes: &out.bytes,
                offset: 0
            }
            .string(128)
            .is_err()
        );
    }

    #[test]
    fn inbound_direction_and_per_kind_limits_precede_payload_allocation() {
        let limits = DecodeLimits {
            frame_bytes: 1024,
            io_bytes: 2,
            path_bytes: 1,
            output_bytes: 3,
            messages: (1 << 3) | (1 << 5) | (1 << 6),
        };
        for message in [
            Message::Call(FsRequest::Write("p".into(), vec![0; 3])),
            Message::Call(FsRequest::Read("pp".into())),
            Message::Reply(Ok(FsValue::Entries(vec![]))),
            Message::Done {
                exit_code: 0,
                stdout: vec![0; 2],
                stderr: vec![0; 2],
                failure: None,
            },
        ] {
            let bytes = Frame {
                session: 1,
                sequence: 1,
                message,
            }
            .encode(1024)
            .unwrap();
            assert!(Frame::decode_bounded(&bytes, limits).is_err());
        }
        let bytes = Frame {
            session: 1,
            sequence: 1,
            message: Message::Call(FsRequest::Write("p".into(), vec![0; 2])),
        }
        .encode(1024)
        .unwrap();
        assert!(Frame::decode_bounded(&bytes, limits).is_ok());
    }
}
