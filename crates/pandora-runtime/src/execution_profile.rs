use crate::containment::shipped_executor_containment;
use crate::executors::WorkspaceRoot;
use pandora_types::{
    ContainmentContractError, ExecutionProfile, ExecutionProfileBinding,
    ExecutionProfileBindingKind, ExecutionProfileContractError,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionProfileAssemblyError {
    InvalidWorkspace,
    Containment(ContainmentContractError),
    UnknownExecutor(String),
    Contract(ExecutionProfileContractError),
}

pub(crate) fn assemble_execution_profile(
    workspace: &WorkspaceRoot,
    policy_version: u32,
    executor_id: &str,
    mut bindings: Vec<ExecutionProfileBinding>,
) -> Result<ExecutionProfile, ExecutionProfileAssemblyError> {
    let workspace_identity = workspace
        .root()
        .to_str()
        .ok_or(ExecutionProfileAssemblyError::InvalidWorkspace)?;
    let containment =
        shipped_executor_containment().map_err(ExecutionProfileAssemblyError::Containment)?;
    let executor = containment
        .executors()
        .iter()
        .find(|evidence| evidence.identity().id() == executor_id)
        .ok_or_else(|| ExecutionProfileAssemblyError::UnknownExecutor(executor_id.to_owned()))?;
    bindings.push(
        ExecutionProfileBinding::new(
            ExecutionProfileBindingKind::Executor,
            executor.identity().id(),
            Some(executor.identity().implementation_version()),
            executor.digest(),
        )
        .map_err(ExecutionProfileAssemblyError::Contract)?,
    );
    ExecutionProfile::new(
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        policy_version,
        workspace_identity,
        containment.digest(),
        bindings,
    )
    .map_err(ExecutionProfileAssemblyError::Contract)
}

#[cfg(test)]
mod tests {
    use super::{ExecutionProfileAssemblyError, assemble_execution_profile};
    use crate::executors::WorkspaceRoot;
    use crate::test_support::new_temp_dir;
    use pandora_types::{ExecutionProfileBinding, ExecutionProfileBindingKind, hash_artifact};

    #[test]
    fn assembly_binds_the_selected_shipped_executor_without_exposing_the_workspace() {
        let directory = new_temp_dir("pandora-execution-profile").unwrap();
        let workspace = WorkspaceRoot::new(&directory).unwrap();
        let gene = ExecutionProfileBinding::new(
            ExecutionProfileBindingKind::Gene,
            "workspace.read",
            Some("0.1.0"),
            hash_artifact(b"workspace.read@0.1.0"),
        )
        .unwrap();

        let profile = assemble_execution_profile(&workspace, 7, "filesystem", vec![gene]).unwrap();
        let json = serde_json::to_string(&profile).unwrap();

        assert_eq!(profile.policy_version(), 7);
        assert_eq!(profile.bindings()[0].id(), "filesystem");
        assert_eq!(profile.bindings()[1].id(), "workspace.read");
        assert!(!json.contains(directory.to_str().unwrap()));

        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn unknown_executor_cannot_produce_a_profile() {
        let directory = new_temp_dir("pandora-execution-profile-unknown").unwrap();
        let workspace = WorkspaceRoot::new(&directory).unwrap();

        assert_eq!(
            assemble_execution_profile(&workspace, 1, "missing", Vec::new()),
            Err(ExecutionProfileAssemblyError::UnknownExecutor(
                "missing".to_owned()
            ))
        );

        let _ = std::fs::remove_dir_all(directory);
    }
}
