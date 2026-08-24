use crate::builtin_genes;
use pandora_types::{GeneId, Harness, HarnessId, PackageKind, PackageManifest};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlashCommandKind {
    Harness,
    Gene,
}

impl SlashCommandKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Harness => "harness",
            Self::Gene => "gene",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashCommand {
    command: String,
    kind: SlashCommandKind,
    harness_id: HarnessId,
    harness_version: String,
    gene_id: Option<GeneId>,
    alias: bool,
}

impl SlashCommand {
    pub fn command(&self) -> &str {
        &self.command
    }

    pub const fn kind(&self) -> SlashCommandKind {
        self.kind
    }

    pub fn harness_id(&self) -> &HarnessId {
        &self.harness_id
    }

    pub fn harness_version(&self) -> &str {
        &self.harness_version
    }

    pub fn gene_id(&self) -> Option<&GeneId> {
        self.gene_id.as_ref()
    }

    pub const fn is_alias(&self) -> bool {
        self.alias
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SlashCommandError {
    DuplicateCommand(String),
    UnsupportedProfile(PackageKind),
}

impl fmt::Display for SlashCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateCommand(command) => {
                write!(formatter, "slash command '{command}' is already registered")
            }
            Self::UnsupportedProfile(kind) => {
                write!(formatter, "{} is not a Harness profile", kind.as_str())
            }
        }
    }
}

impl std::error::Error for SlashCommandError {}

pub struct SlashCommandCatalog {
    commands: BTreeMap<String, SlashCommand>,
}

impl SlashCommandCatalog {
    pub fn from_harnesses<'a>(
        harnesses: impl IntoIterator<Item = &'a dyn Harness>,
    ) -> Result<Self, SlashCommandError> {
        let mut catalog = Self {
            commands: BTreeMap::new(),
        };
        for harness in harnesses {
            catalog.add_harness(harness)?;
        }
        Ok(catalog)
    }

    pub fn add_profile(&mut self, package: &PackageManifest) -> Result<(), SlashCommandError> {
        if !matches!(
            package.kind(),
            PackageKind::DomainHarness | PackageKind::MetaHarness
        ) {
            return Err(SlashCommandError::UnsupportedProfile(package.kind()));
        }
        let harness_id = HarnessId::new(package.id().as_str().to_owned())
            .expect("validated package ID is a valid Harness ID");
        self.add_command(SlashCommand {
            command: canonical_profile_harness_command(harness_id.as_str(), package.version()),
            kind: SlashCommandKind::Harness,
            harness_id: harness_id.clone(),
            harness_version: package.version().to_owned(),
            gene_id: None,
            alias: false,
        })?;
        if package.kind() == PackageKind::DomainHarness {
            let available_genes = builtin_genes();
            for dependency in package.dependencies() {
                if !available_genes.iter().any(|gene| {
                    gene.manifest().id().as_str() == dependency.id().as_str()
                        && gene.manifest().version() == dependency.version()
                }) {
                    continue;
                }
                self.add_gene(
                    &harness_id,
                    package.version(),
                    GeneId::new(dependency.id().as_str().to_owned())
                        .expect("validated dependency ID is a valid Gene ID"),
                    false,
                    true,
                )?;
            }
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<&SlashCommand> {
        self.commands.values().collect()
    }

    pub fn resolve(&self, command: &str) -> Option<&SlashCommand> {
        self.commands.get(command)
    }

    fn add_harness(&mut self, harness: &dyn Harness) -> Result<(), SlashCommandError> {
        let harness_id = harness.manifest().id().clone();
        self.add_command(SlashCommand {
            command: canonical_harness_command(harness_id.as_str()),
            kind: SlashCommandKind::Harness,
            harness_id: harness_id.clone(),
            harness_version: harness.manifest().version().to_owned(),
            gene_id: None,
            alias: false,
        })?;
        if let Some(alias) = built_in_harness_alias(harness_id.as_str()) {
            self.add_command(SlashCommand {
                command: alias.to_owned(),
                kind: SlashCommandKind::Harness,
                harness_id: harness_id.clone(),
                harness_version: harness.manifest().version().to_owned(),
                gene_id: None,
                alias: true,
            })?;
        }
        for gene in harness.genes() {
            self.add_gene(
                &harness_id,
                harness.manifest().version(),
                gene.manifest().id().clone(),
                true,
                false,
            )?;
        }
        Ok(())
    }

    fn add_gene(
        &mut self,
        harness_id: &HarnessId,
        harness_version: &str,
        gene_id: GeneId,
        allow_core_alias: bool,
        version_qualified: bool,
    ) -> Result<(), SlashCommandError> {
        self.add_command(SlashCommand {
            command: if version_qualified {
                canonical_profile_gene_command(
                    harness_id.as_str(),
                    harness_version,
                    gene_id.as_str(),
                )
            } else {
                canonical_gene_command(harness_id.as_str(), gene_id.as_str())
            },
            kind: SlashCommandKind::Gene,
            harness_id: harness_id.clone(),
            harness_version: harness_version.to_owned(),
            gene_id: Some(gene_id.clone()),
            alias: false,
        })?;
        if allow_core_alias && let Some(alias) = built_in_gene_alias(gene_id.as_str()) {
            self.add_command(SlashCommand {
                command: alias.to_owned(),
                kind: SlashCommandKind::Gene,
                harness_id: harness_id.clone(),
                harness_version: harness_version.to_owned(),
                gene_id: Some(gene_id),
                alias: true,
            })?;
        }
        Ok(())
    }

    fn add_command(&mut self, command: SlashCommand) -> Result<(), SlashCommandError> {
        if self.commands.contains_key(command.command()) {
            return Err(SlashCommandError::DuplicateCommand(
                command.command().to_owned(),
            ));
        }
        self.commands.insert(command.command.clone(), command);
        Ok(())
    }
}

pub fn canonical_harness_command(harness_id: &str) -> String {
    format!("/harness:{}", encode_segment(harness_id))
}

pub fn canonical_gene_command(harness_id: &str, gene_id: &str) -> String {
    format!(
        "/gene:{}:{}",
        encode_segment(harness_id),
        encode_segment(gene_id)
    )
}

pub fn canonical_profile_harness_command(harness_id: &str, version: &str) -> String {
    format!(
        "/harness:{}@{}",
        encode_segment(harness_id),
        encode_segment(version)
    )
}

pub fn canonical_profile_gene_command(harness_id: &str, version: &str, gene_id: &str) -> String {
    format!(
        "/gene:{}@{}:{}",
        encode_segment(harness_id),
        encode_segment(version),
        encode_segment(gene_id)
    )
}

fn encode_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn built_in_harness_alias(id: &str) -> Option<&'static str> {
    match id {
        "core-source" => Some("/core"),
        "coding-domain" => Some("/coding"),
        "research-domain" => Some("/research"),
        "coordination-meta" => Some("/coordination"),
        _ => None,
    }
}

fn built_in_gene_alias(id: &str) -> Option<&'static str> {
    match id {
        "workspace.read" => Some("/read"),
        "workspace.search" => Some("/search"),
        "patch.apply" => Some("/patch"),
        "verification.run" => Some("/verify"),
        "change.review" => Some("/review"),
        "daedalus.audit" => Some("/audit"),
        "argus.review" => Some("/argus-review"),
        "ariadne.debt" => Some("/debt"),
        "hephaestus.measure" => Some("/measure"),
        "athena.guide" => Some("/guide"),
        "evidence.inventory" => Some("/evidence-inventory"),
        "evidence.search" => Some("/evidence-search"),
        "source.read" => Some("/source-read"),
        "source.compare" => Some("/source-compare"),
        "citation.inventory" => Some("/citation-inventory"),
        "research.guide" => Some("/research-guide"),
        _ => None,
    }
}
