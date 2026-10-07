//! The report channel between the helper and the parent.
//!
//! The helper applies confinement, proves it, writes a [`ConfinementOutcome`]
//! to the report descriptor, and only then executes the target. The parent
//! reads that report and refuses to proceed unless it arrived, parsed, and
//! satisfies the profile.
//!
//! The framing is a fixed-width hex length followed by that many bytes of JSON.
//! Fixed width rather than a newline delimiter so a helper that dies mid-write
//! leaves the parent reading a short buffer and refusing, instead of waiting
//! forever or accepting a truncated report.

use crate::error::SandboxProtocolError;
use crate::outcome::CONFINEMENT_OUTCOME_VERSION;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fmt::Write as _;

/// Width of the length prefix, in hex characters.
pub const REPORT_LENGTH_HEX: usize = 8;

/// Largest report accepted, so a confused or hostile helper cannot make the
/// parent allocate without bound.
pub const MAX_REPORT_BYTES: usize = 64 * 1024;

/// What the helper writes back before it executes anything.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperReport {
    version: u16,
    outcome: crate::outcome::ConfinementOutcome,
    /// Identity the helper actually ran as, when it created one. Empty when it
    /// did not, which is every platform in STEP 1.
    restricted_identity: Option<String>,
}

impl HelperReport {
    pub fn new(
        outcome: crate::outcome::ConfinementOutcome,
        restricted_identity: Option<String>,
    ) -> Self {
        Self {
            version: CONFINEMENT_OUTCOME_VERSION,
            outcome,
            restricted_identity,
        }
    }

    pub const fn version(&self) -> u16 {
        self.version
    }

    pub const fn outcome(&self) -> &crate::outcome::ConfinementOutcome {
        &self.outcome
    }

    pub fn restricted_identity(&self) -> Option<&str> {
        self.restricted_identity.as_deref()
    }
}

/// Encodes a report into the length-prefixed frame.
pub fn encode_report(report: &HelperReport) -> Result<Vec<u8>, SandboxProtocolError> {
    let json = serde_json::to_vec(report).map_err(|_| SandboxProtocolError::Unserializable)?;
    if json.len() > MAX_REPORT_BYTES {
        return Err(SandboxProtocolError::ReportTooLarge {
            len: json.len(),
            limit: MAX_REPORT_BYTES,
        });
    }
    let mut framed = Vec::with_capacity(REPORT_LENGTH_HEX + json.len());
    let mut prefix = String::with_capacity(REPORT_LENGTH_HEX);
    // write! to a String cannot fail, but a hand-rolled pad avoids ignoring a
    // Result at the one place a framing bug would be silent.
    let _ = write!(prefix, "{:0width$x}", json.len(), width = REPORT_LENGTH_HEX);
    framed.extend_from_slice(prefix.as_bytes());
    framed.extend_from_slice(&json);
    Ok(framed)
}

/// Decodes a frame. Rejects a short buffer rather than treating it as an empty
/// report, so a helper that dies mid-write cannot look like a refusal.
pub fn decode_report(frame: &[u8]) -> Result<HelperReport, SandboxProtocolError> {
    if frame.len() < REPORT_LENGTH_HEX {
        return Err(SandboxProtocolError::Truncated {
            len: frame.len(),
            need: REPORT_LENGTH_HEX,
        });
    }
    let prefix = std::str::from_utf8(&frame[..REPORT_LENGTH_HEX])
        .map_err(|_| SandboxProtocolError::MalformedLength)?;
    let declared =
        usize::from_str_radix(prefix, 16).map_err(|_| SandboxProtocolError::MalformedLength)?;
    if declared == 0 {
        return Err(SandboxProtocolError::EmptyReport);
    }
    if declared > MAX_REPORT_BYTES {
        return Err(SandboxProtocolError::ReportTooLarge {
            len: declared,
            limit: MAX_REPORT_BYTES,
        });
    }
    let body = &frame[REPORT_LENGTH_HEX..];
    if body.len() < declared {
        return Err(SandboxProtocolError::Truncated {
            len: body.len(),
            need: declared,
        });
    }
    let report: HelperReport =
        serde_json::from_slice(&body[..declared]).map_err(|_| SandboxProtocolError::Unparsable)?;
    if report.version != CONFINEMENT_OUTCOME_VERSION {
        return Err(SandboxProtocolError::VersionMismatch {
            found: report.version,
            expected: CONFINEMENT_OUTCOME_VERSION,
        });
    }
    Ok(report)
}

impl fmt::Display for SandboxProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unserializable => {
                formatter.write_str("the helper report could not be serialized")
            }
            Self::Unparsable => formatter.write_str("the helper report could not be parsed"),
            Self::Truncated { len, need } => {
                write!(
                    formatter,
                    "the helper report is truncated: got {len} bytes, need {need}"
                )
            }
            Self::MalformedLength => {
                formatter.write_str("the helper report length prefix is malformed")
            }
            Self::EmptyReport => formatter.write_str("the helper reported nothing"),
            Self::ReportTooLarge { len, limit } => {
                write!(
                    formatter,
                    "the helper report is {len} bytes, over the {limit} limit"
                )
            }
            Self::VersionMismatch { found, expected } => {
                write!(
                    formatter,
                    "helper report version {found} is not the expected {expected}"
                )
            }
        }
    }
}
