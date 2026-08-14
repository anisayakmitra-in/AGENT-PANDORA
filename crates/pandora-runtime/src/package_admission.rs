use pandora_types::SkillManifest;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PackageAdmissionError {
    InvalidResource,
}

impl fmt::Display for PackageAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidResource => formatter.write_str("skill resource is invalid"),
        }
    }
}

impl std::error::Error for PackageAdmissionError {}

pub struct PackageAdmission;

impl PackageAdmission {
    pub fn validate_skill(manifest: &SkillManifest) -> Result<(), PackageAdmissionError> {
        if manifest.resources().iter().any(|resource| {
            resource.is_empty()
                || resource.chars().any(char::is_control)
                || resource == "."
                || resource == ".."
                || resource.contains(['/', '\\'])
        }) {
            return Err(PackageAdmissionError::InvalidResource);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_labels_cannot_be_paths() {
        let manifest = SkillManifest::new(
            "alpha",
            "0.1.0",
            "Alpha",
            "A skill",
            None,
            vec!["../secrets".to_owned()],
        )
        .unwrap();

        assert_eq!(
            PackageAdmission::validate_skill(&manifest),
            Err(PackageAdmissionError::InvalidResource)
        );
    }
}
