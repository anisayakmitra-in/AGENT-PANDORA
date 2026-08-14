use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdError {
    Empty,
    TooLong,
}

impl fmt::Display for IdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("identifier cannot be empty"),
            Self::TooLong => formatter.write_str("identifier exceeds the 256-byte limit"),
        }
    }
}

impl std::error::Error for IdError {}

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(IdError::Empty);
                }
                if value.len() > 256 {
                    return Err(IdError::TooLong);
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

define_id!(ExecutionId);
define_id!(SessionId);
define_id!(PrincipalId);
define_id!(GeneId);
define_id!(HarnessId);
define_id!(PackageId);
define_id!(ArtifactId);
define_id!(PermitId);
define_id!(ReceiptId);
define_id!(RequestDigest);
define_id!(EventId);
define_id!(TenantId);
define_id!(WorkspaceId);
define_id!(MemoryId);
