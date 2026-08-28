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
    ($name:ident, validated_deserialize) => {
        #[derive(Clone, Debug, Serialize, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<Deserializer>(
                deserializer: Deserializer,
            ) -> Result<Self, Deserializer::Error>
            where
                Deserializer: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
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
define_id!(PlanId);
define_id!(RoleId);
define_id!(RunLoopId);
define_id!(ProposalId);
define_id!(FailureId);
define_id!(PopulationId);
define_id!(JobId);
define_id!(JobWorkerId);
define_id!(SubagentId, validated_deserialize);
define_id!(RepositoryId, validated_deserialize);
define_id!(OrchestrationRunId, validated_deserialize);
