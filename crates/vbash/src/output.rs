use vsh_monty::MontyObject;

/// Guest language selected explicitly for one execution request.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Language {
    /// The bounded Python interpreter used by existing requests.
    #[default]
    Monty,
    /// The separately isolated, explicitly enabled Bashkit frontend.
    Bash,
}

/// Complete byte-authoritative output from a successful Bash execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BashResult {
    /// Compatibility profile used for this execution, sealed with the output.
    pub profile: String,
    /// Final shell status; successful simulation results always have status zero.
    pub exit_code: i32,
    /// Complete bounded stdout, including non-UTF-8 bytes.
    pub stdout: Vec<u8>,
    /// Complete bounded stderr, including non-UTF-8 bytes.
    pub stderr: Vec<u8>,
}

/// One canonical owned output envelope, shared by persistence and review.
#[derive(Clone, Debug, PartialEq)]
pub enum ExecutionOutput {
    /// A faithfully represented Monty value and captured print output.
    Monty {
        /// Bounded native return value.
        value: MontyObject,
        /// Bounded UTF-8 print output.
        stdout: String,
    },
    /// A complete successful Bash execution with byte streams.
    Bash(BashResult),
}

impl ExecutionOutput {
    /// Return the language from its explicit result tag.
    #[must_use]
    pub const fn language(&self) -> Language {
        match self {
            Self::Monty { .. } => Language::Monty,
            Self::Bash(_) => Language::Bash,
        }
    }

    /// Return the native Monty value, if this is a Monty execution.
    #[must_use]
    pub const fn monty_value(&self) -> Option<&MontyObject> {
        match self {
            Self::Monty { value, .. } => Some(value),
            Self::Bash(_) => None,
        }
    }

    /// Return complete stdout bytes without a lossy text conversion.
    #[must_use]
    pub fn stdout_bytes(&self) -> &[u8] {
        match self {
            Self::Monty { stdout, .. } => stdout.as_bytes(),
            Self::Bash(result) => &result.stdout,
        }
    }

    /// Return complete stderr bytes; Monty exceptions use the error surface.
    #[must_use]
    pub fn stderr_bytes(&self) -> &[u8] {
        match self {
            Self::Monty { .. } => &[],
            Self::Bash(result) => &result.stderr,
        }
    }
}
