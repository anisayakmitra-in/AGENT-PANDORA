use serde::{Deserialize, Serialize};

const fn default_feature_map_version() -> u32 {
    1
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureMap {
    #[serde(default = "default_feature_map_version")]
    pub version: u32,
    pub features: Vec<FeatureEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureEntry {
    pub name: String,
    pub entry_point: String,
    pub description: String,
    pub invariants: Vec<String>,
    pub dependencies: Vec<String>,
}

impl FeatureMap {
    pub fn new(features: Vec<FeatureEntry>) -> Self {
        Self {
            version: 1,
            features,
        }
    }

    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn features(&self) -> &[FeatureEntry] {
        &self.features
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version == 0 {
            return Err("feature map version must be greater than zero");
        }
        if self.features.is_empty() {
            return Err("feature map must contain at least one feature");
        }
        for feature in &self.features {
            if feature.name.trim().is_empty() {
                return Err("feature name cannot be empty");
            }
            if feature.entry_point.trim().is_empty() {
                return Err("feature entry point cannot be empty");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_map_validates_cleanly() {
        let map = FeatureMap::new(vec![FeatureEntry {
            name: "Parliament".to_owned(),
            entry_point: "crates/pandora-runtime/src/parliament.rs".to_owned(),
            description: "Governance and approval authority".to_owned(),
            invariants: vec!["Human review required for high-risk actions".to_owned()],
            dependencies: vec!["ReferenceMonitor".to_owned()],
        }]);

        assert_eq!(map.validate(), Ok(()));
        let json = serde_json::to_string(&map).unwrap();
        let round_trip: FeatureMap = serde_json::from_str(&json).unwrap();
        assert_eq!(map, round_trip);
    }

    #[test]
    fn feature_map_rejects_empty_features() {
        let map = FeatureMap {
            version: 1,
            features: Vec::new(),
        };
        assert_eq!(
            map.validate(),
            Err("feature map must contain at least one feature")
        );
    }
}
