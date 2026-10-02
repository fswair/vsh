use std::error::Error;
use std::fmt;
use vsh_types::{VPath, VPathError};

/// Default absolute namespace prefix for the workspace.
pub const DEFAULT_VIRTUAL_ROOT: &str = "/workspace";

/// A validated absolute namespace prefix exposed to guest code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VirtualRoot {
    absolute: String,
}

impl VirtualRoot {
    /// Validate and construct a synthetic absolute workspace root.
    ///
    /// # Errors
    ///
    /// Returns [`VirtualRootError`] unless `absolute` is a normalized POSIX-style
    /// absolute path without NUL, parent, platform-prefix, or backslash components.
    pub fn new(absolute: impl Into<String>) -> Result<Self, VirtualRootError> {
        let absolute = absolute.into();
        if absolute.contains('\0') {
            return Err(VirtualRootError::NulByte);
        }
        if !absolute.starts_with('/') {
            return Err(VirtualRootError::NotAbsolute);
        }
        if absolute.contains('\\') {
            return Err(VirtualRootError::PlatformSeparator);
        }

        let mut components = Vec::new();
        for component in absolute.split('/') {
            match component {
                "" | "." => {}
                ".." => return Err(VirtualRootError::ParentComponent),
                value if is_windows_prefix(value) => {
                    return Err(VirtualRootError::PlatformPrefix);
                }
                value => components.push(value),
            }
        }
        let absolute = if components.is_empty() {
            "/".to_owned()
        } else {
            format!("/{}", components.join("/"))
        };
        Ok(Self { absolute })
    }

    /// Return the canonical absolute virtual prefix.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.absolute
    }

    /// Map a guest-visible path into the relative VSH namespace.
    ///
    /// # Errors
    ///
    /// Returns [`VirtualPathError`] when the input is malformed or outside this root.
    pub fn map_path(&self, input: &str) -> Result<VPath, VirtualPathError> {
        if input.is_empty() {
            return Err(VirtualPathError::Empty);
        }
        if input.contains('\0') {
            return Err(VirtualPathError::NulByte);
        }

        let portable = input.replace('\\', "/");
        if portable.starts_with('/') {
            let absolute = normalize_absolute(&portable)?;
            let relative = if self.absolute == "/" {
                absolute.strip_prefix('/').unwrap_or(&absolute)
            } else if absolute == self.absolute {
                ""
            } else {
                absolute
                    .strip_prefix(&self.absolute)
                    .and_then(|suffix| suffix.strip_prefix('/'))
                    .ok_or(VirtualPathError::OutsideRoot)?
            };
            if relative.is_empty() {
                Ok(VPath::root())
            } else {
                VPath::parse(relative).map_err(VirtualPathError::InvalidRelative)
            }
        } else {
            VPath::parse(&portable).map_err(VirtualPathError::InvalidRelative)
        }
    }

    /// Present a relative path in the configured absolute namespace.
    #[must_use]
    pub fn present(&self, path: &VPath) -> String {
        if path.is_root() {
            return self.absolute.clone();
        }
        if self.absolute == "/" {
            format!("/{}", path.as_str())
        } else {
            format!("{}/{}", self.absolute, path.as_str())
        }
    }
}

impl Default for VirtualRoot {
    fn default() -> Self {
        Self {
            absolute: DEFAULT_VIRTUAL_ROOT.to_owned(),
        }
    }
}

/// Invalid synthetic-root configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum VirtualRootError {
    /// The configured root was relative.
    NotAbsolute,
    /// The root contained a parent component.
    ParentComponent,
    /// The root contained a NUL byte.
    NulByte,
    /// The root used a platform-specific separator.
    PlatformSeparator,
    /// The root contained a drive-style component.
    PlatformPrefix,
}

impl fmt::Display for VirtualRootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotAbsolute => "virtual root must be absolute",
            Self::ParentComponent => "virtual root contains a parent component",
            Self::NulByte => "virtual root contains a NUL byte",
            Self::PlatformSeparator => "virtual root contains a platform separator",
            Self::PlatformPrefix => "virtual root contains a platform prefix",
        })
    }
}

impl Error for VirtualRootError {}

/// A guest path that cannot name a node in the configured virtual root.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum VirtualPathError {
    /// The supplied path was empty.
    Empty,
    /// The supplied path contained a NUL byte.
    NulByte,
    /// Absolute normalization attempted to move above `/`.
    EscapesAbsoluteRoot,
    /// The normalized absolute path was outside the configured VSH root.
    OutsideRoot,
    /// Relative VSH path validation rejected the value.
    InvalidRelative(VPathError),
}

impl fmt::Display for VirtualPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("virtual path must not be empty"),
            Self::NulByte => formatter.write_str("virtual path contains a NUL byte"),
            Self::EscapesAbsoluteRoot => formatter.write_str("virtual path escapes absolute root"),
            Self::OutsideRoot => formatter.write_str("virtual path is outside /workspace"),
            Self::InvalidRelative(source) => {
                write!(formatter, "invalid relative virtual path: {source}")
            }
        }
    }
}

impl Error for VirtualPathError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidRelative(source) => Some(source),
            Self::Empty | Self::NulByte | Self::EscapesAbsoluteRoot | Self::OutsideRoot => None,
        }
    }
}

fn normalize_absolute(input: &str) -> Result<String, VirtualPathError> {
    let mut components = Vec::new();
    for component in input.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return Err(VirtualPathError::EscapesAbsoluteRoot);
                }
            }
            value => components.push(value),
        }
    }
    if components.is_empty() {
        Ok("/".to_owned())
    } else {
        Ok(format!("/{}", components.join("/")))
    }
}

fn is_windows_prefix(component: &str) -> bool {
    let bytes = component.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}
