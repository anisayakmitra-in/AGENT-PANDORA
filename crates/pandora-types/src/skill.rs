use std::fmt;

const MAX_SKILL_ID_BYTES: usize = 128;
const MAX_SKILL_TEXT_BYTES: usize = 4096;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SkillId(String);

impl SkillId {
    pub fn new(value: impl Into<String>) -> Result<Self, SkillManifestError> {
        let value = value.into();
        if value.is_empty() {
            return Err(SkillManifestError::EmptyField("id"));
        }
        if value.len() > MAX_SKILL_ID_BYTES {
            return Err(SkillManifestError::FieldTooLong("id"));
        }
        if matches!(value.as_str(), "." | "..")
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(SkillManifestError::InvalidId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SkillId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillManifestError {
    EmptyField(&'static str),
    FieldTooLong(&'static str),
    InvalidId,
    ControlCharacter(&'static str),
}

impl fmt::Display for SkillManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(formatter, "{field} cannot be empty"),
            Self::FieldTooLong(field) => write!(formatter, "{field} is too long"),
            Self::InvalidId => formatter.write_str("skill id contains an invalid character"),
            Self::ControlCharacter(field) => {
                write!(formatter, "{field} contains a control character")
            }
        }
    }
}

impl std::error::Error for SkillManifestError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillManifest {
    id: SkillId,
    version: String,
    name: String,
    description: String,
    publisher: Option<String>,
    resources: Vec<String>,
}

impl SkillManifest {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        publisher: Option<String>,
        resources: Vec<String>,
    ) -> Result<Self, SkillManifestError> {
        let id = SkillId::new(id)?;
        let version = validate_text("version", version.into())?;
        let name = validate_text("name", name.into())?;
        let description = validate_text("description", description.into())?;
        let publisher = publisher
            .map(|publisher| validate_text("publisher", publisher))
            .transpose()?;
        let resources = resources
            .into_iter()
            .map(|resource| validate_text("resource", resource))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            id,
            version,
            name,
            description,
            publisher,
            resources,
        })
    }

    pub fn id(&self) -> &SkillId {
        &self.id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn publisher(&self) -> Option<&str> {
        self.publisher.as_deref()
    }

    pub fn resources(&self) -> &[String] {
        &self.resources
    }
}

fn validate_text(field: &'static str, value: String) -> Result<String, SkillManifestError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(SkillManifestError::EmptyField(field));
    }
    if value.len() > MAX_SKILL_TEXT_BYTES {
        return Err(SkillManifestError::FieldTooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(SkillManifestError::ControlCharacter(field));
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_ids_reject_path_components() {
        assert_eq!(SkillId::new("../alpha"), Err(SkillManifestError::InvalidId));
    }

    #[test]
    fn skill_ids_reject_dot_segments() {
        assert_eq!(SkillId::new("."), Err(SkillManifestError::InvalidId));
        assert_eq!(SkillId::new(".."), Err(SkillManifestError::InvalidId));
    }

    #[test]
    fn manifest_preserves_validated_metadata() {
        let manifest = SkillManifest::new(
            "alpha",
            "0.1.0",
            " Alpha Skill ",
            " Reads files ",
            Some("pandora".to_owned()),
            vec!["workspace.read".to_owned()],
        )
        .unwrap();

        assert_eq!(manifest.id().as_str(), "alpha");
        assert_eq!(manifest.name(), "Alpha Skill");
        assert_eq!(manifest.publisher(), Some("pandora"));
    }
}
