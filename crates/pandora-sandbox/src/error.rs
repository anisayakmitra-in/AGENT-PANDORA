//! Fail-closed errors. There is no variant that means "carry on unconfined".

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxProfileError {
    TooManyWritableRoots { count: usize, limit: usize },
    EmptyProfileVersion,
    UnknownVersion { found: u16, expected: u16 },
}

impl fmt::Display for SandboxProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyWritableRoots { count, limit } => {
                write!(
                    formatter,
                    "{count} writable roots exceeds the limit of {limit}"
                )
            }
            Self::EmptyProfileVersion => formatter.write_str("the sandbox profile has no version"),
            Self::UnknownVersion { found, expected } => {
                write!(
                    formatter,
                    "sandbox profile version {found} is not supported, expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for SandboxProfileError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxProtocolError {
    Unserializable,
    Unparsable,
    Truncated { len: usize, need: usize },
    MalformedLength,
    EmptyReport,
    ReportTooLarge { len: usize, limit: usize },
    VersionMismatch { found: u16, expected: u16 },
}

impl std::error::Error for SandboxProtocolError {}
