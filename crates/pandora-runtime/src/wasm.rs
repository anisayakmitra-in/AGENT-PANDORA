use crate::ConsumedPermit;
use pandora_types::{
    ArtifactId, Capability, EffectOutcome, EffectReceipt, EffectTarget, ExecutionId,
    ExecutionProfile, ExecutionProfileBindingKind, Gene, GeneError, GeneInput, GeneKind,
    GeneManifest, Operation, OperationRequest, PackageKind, PackageManifest, PrincipalId,
    ReceiptId, ResourceScope, SessionId, Timestamp, hash_artifact,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use wasmi::{
    Config, EnforcedLimits, Engine, ExternType, Linker, Module, Store, StoreLimits,
    StoreLimitsBuilder, TrapCode, ValType,
};

pub const MAX_WASM_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_WASM_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_WASM_MEMORY_BYTES: usize = 16 * 1024 * 1024;
pub const DEFAULT_WASM_FUEL: u64 = 1_000_000;

static NEXT_RECEIPT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WasmError {
    InvalidPackageKind,
    ArtifactHashMismatch,
    DuplicatePackage,
    InvalidModule,
    ImportsForbidden,
    InvalidAbi,
    UnknownPackage,
    InputTooLarge,
    OutputTooLarge,
    InvalidInput,
    InvalidOutput,
    PermissionDenied,
    ResourceLimit,
    ExecutionFailed,
    InvalidManifest,
}

impl WasmError {
    fn code(&self) -> &'static str {
        match self {
            Self::InvalidPackageKind => "wasm_invalid_package_kind",
            Self::ArtifactHashMismatch => "wasm_artifact_hash_mismatch",
            Self::DuplicatePackage => "wasm_duplicate_package",
            Self::InvalidModule => "wasm_invalid_module",
            Self::ImportsForbidden => "wasm_imports_forbidden",
            Self::InvalidAbi => "wasm_invalid_abi",
            Self::UnknownPackage => "wasm_unknown_package",
            Self::InputTooLarge => "wasm_input_too_large",
            Self::OutputTooLarge => "wasm_output_too_large",
            Self::InvalidInput => "wasm_invalid_input",
            Self::InvalidOutput => "wasm_invalid_output",
            Self::PermissionDenied => "wasm_permission_denied",
            Self::ResourceLimit => "wasm_resource_limit",
            Self::ExecutionFailed => "wasm_execution_failed",
            Self::InvalidManifest => "wasm_invalid_manifest",
        }
    }
}

impl fmt::Display for WasmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for WasmError {}

pub struct WasmResult {
    result: Result<Vec<u8>, WasmError>,
    receipt: EffectReceipt,
}

impl WasmResult {
    pub fn result(&self) -> Result<&Vec<u8>, &WasmError> {
        self.result.as_ref()
    }

    pub fn into_result(self) -> Result<Vec<u8>, WasmError> {
        self.result
    }

    pub fn receipt(&self) -> &EffectReceipt {
        &self.receipt
    }
}

struct RegisteredModule {
    module: Module,
    content_hash: String,
}

struct StoreState {
    limits: StoreLimits,
}

pub struct WasmExecutor {
    engine: Engine,
    modules: BTreeMap<(String, String), RegisteredModule>,
}

pub struct WasmGene {
    manifest: GeneManifest,
    artifact_id: ArtifactId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WasmGeneRequest {
    execution_id: ExecutionId,
    session_id: SessionId,
    principal_id: PrincipalId,
    execution_profile: ExecutionProfile,
    payload: String,
}

impl WasmGeneRequest {
    pub fn new(
        execution_id: ExecutionId,
        session_id: SessionId,
        principal_id: PrincipalId,
        execution_profile: ExecutionProfile,
        payload: impl Into<String>,
    ) -> Result<Self, GeneError> {
        let request = Self {
            execution_id,
            session_id,
            principal_id,
            execution_profile,
            payload: payload.into(),
        };
        request.validate()?;
        Ok(request)
    }

    pub fn payload(&self) -> &[u8] {
        self.payload.as_bytes()
    }

    pub fn into_gene_input(self) -> Result<GeneInput, GeneError> {
        self.validate()?;
        let value = serde_json::to_string(&self)
            .map_err(|_| GeneError::InvalidInput("Wasm Gene input could not be encoded"))?;
        GeneInput::new(value)
    }

    fn parse(input: &GeneInput) -> Result<Self, GeneError> {
        let request: Self = serde_json::from_str(input.as_str())
            .map_err(|_| GeneError::InvalidInput("Wasm Gene input must be valid JSON"))?;
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), GeneError> {
        if self.payload.len() > MAX_WASM_INPUT_BYTES {
            return Err(GeneError::InvalidInput("Wasm Gene input exceeds the limit"));
        }
        serde_json::from_str::<serde_json::Value>(&self.payload)
            .map_err(|_| GeneError::InvalidInput("Wasm Gene payload must be valid JSON"))?;
        Ok(())
    }
}

impl WasmGene {
    pub fn from_package(package: &PackageManifest) -> Result<Self, WasmError> {
        if package.kind() != PackageKind::Gene {
            return Err(WasmError::InvalidPackageKind);
        }
        let manifest = GeneManifest::new(
            package.id().as_str(),
            package.version(),
            GeneKind::Tool,
            vec![Capability::WasmExecute],
        )
        .map_err(|_| WasmError::InvalidManifest)?;
        let artifact_id =
            ArtifactId::new(package.content_hash()).map_err(|_| WasmError::InvalidManifest)?;
        Ok(Self {
            manifest,
            artifact_id,
        })
    }
}

impl Gene for WasmGene {
    fn manifest(&self) -> &GeneManifest {
        &self.manifest
    }

    fn plan(&self, input: &GeneInput) -> Result<Vec<OperationRequest>, GeneError> {
        let request = WasmGeneRequest::parse(input)?;
        let operation = OperationRequest::new(
            request.execution_id,
            request.session_id,
            request.principal_id,
            request.execution_profile,
            self.manifest.id().clone(),
            Some(self.artifact_id.clone()),
            Capability::WasmExecute,
            Operation::Execute,
            EffectTarget::wasm(self.manifest.id().as_str(), self.manifest.version()),
            ResourceScope::none(),
        )?
        .with_payload_digest(request.payload.as_bytes())?;
        Ok(vec![operation])
    }
}

impl WasmExecutor {
    pub fn new() -> Self {
        let mut config = Config::default();
        config
            .consume_fuel(true)
            .enforced_limits(EnforcedLimits::strict())
            .ignore_custom_sections(true);
        Self {
            engine: Engine::new(&config),
            modules: BTreeMap::new(),
        }
    }

    pub fn register(
        &mut self,
        manifest: &PackageManifest,
        artifact: &[u8],
    ) -> Result<(), WasmError> {
        if manifest.kind() != PackageKind::Gene {
            return Err(WasmError::InvalidPackageKind);
        }
        if hash_artifact(artifact) != manifest.content_hash() {
            return Err(WasmError::ArtifactHashMismatch);
        }
        let key = (
            manifest.id().as_str().to_owned(),
            manifest.version().to_owned(),
        );
        if self.modules.contains_key(&key) {
            return Err(WasmError::DuplicatePackage);
        }
        let module = Module::new(&self.engine, artifact).map_err(|_| WasmError::InvalidModule)?;
        if module.imports().next().is_some() {
            return Err(WasmError::ImportsForbidden);
        }
        validate_abi(&module)?;
        self.modules.insert(
            key,
            RegisteredModule {
                module,
                content_hash: manifest.content_hash().to_owned(),
            },
        );
        Ok(())
    }

    pub fn content_hash(&self, package_id: &str, version: &str) -> Option<&str> {
        self.modules
            .get(&(package_id.to_owned(), version.to_owned()))
            .map(|module| module.content_hash.as_str())
    }

    pub fn execute(&self, permit: &ConsumedPermit, input: &[u8], now: Timestamp) -> WasmResult {
        let result = self.execute_inner(permit, input);
        let outcome = match &result {
            Ok(_) => EffectOutcome::Succeeded,
            Err(error) => EffectOutcome::Failed {
                code: error.code().to_owned(),
            },
        };
        WasmResult {
            result,
            receipt: receipt_for(permit, now, outcome),
        }
    }

    fn execute_inner(&self, permit: &ConsumedPermit, input: &[u8]) -> Result<Vec<u8>, WasmError> {
        if input.len() > MAX_WASM_INPUT_BYTES {
            return Err(WasmError::InputTooLarge);
        }
        serde_json::from_slice::<serde_json::Value>(input).map_err(|_| WasmError::InvalidInput)?;

        let request = permit.request();
        let EffectTarget::Wasm {
            package_id,
            version,
        } = request.target()
        else {
            return Err(WasmError::PermissionDenied);
        };
        let Some(registered) = self.modules.get(&(package_id.clone(), version.clone())) else {
            return Err(WasmError::UnknownPackage);
        };
        let artifact_matches = request
            .execution_profile()
            .bindings()
            .iter()
            .any(|binding| {
                binding.kind() == ExecutionProfileBindingKind::Artifact
                    && binding.id() == package_id
                    && binding.version() == Some(version.as_str())
                    && binding.digest() == registered.content_hash
            });
        let executor_matches = request
            .execution_profile()
            .bindings()
            .iter()
            .any(|binding| {
                binding.kind() == ExecutionProfileBindingKind::Executor && binding.id() == "wasm"
            });
        if request.capability() != Capability::WasmExecute
            || request.operation() != Operation::Execute
            || request.resource_scope() != &ResourceScope::none()
            || request.gene_id().as_str() != package_id
            || request.artifact_id().map(|id| id.as_str()) != Some(registered.content_hash.as_str())
            || !request.payload_digest_matches(input)
            || !artifact_matches
            || !executor_matches
        {
            return Err(WasmError::PermissionDenied);
        }

        run_module(&self.engine, &registered.module, input)
    }
}

impl Default for WasmExecutor {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_abi(module: &Module) -> Result<(), WasmError> {
    if !matches!(module.get_export("memory"), Some(ExternType::Memory(_))) {
        return Err(WasmError::InvalidAbi);
    }
    let Some(ExternType::Func(allocate)) = module.get_export("pandora_alloc") else {
        return Err(WasmError::InvalidAbi);
    };
    if allocate.params() != [ValType::I32].as_slice()
        || allocate.results() != [ValType::I32].as_slice()
    {
        return Err(WasmError::InvalidAbi);
    }
    let Some(ExternType::Func(run)) = module.get_export("pandora_run") else {
        return Err(WasmError::InvalidAbi);
    };
    if run.params() != [ValType::I32, ValType::I32].as_slice()
        || run.results() != [ValType::I64].as_slice()
    {
        return Err(WasmError::InvalidAbi);
    }
    Ok(())
}

fn run_module(engine: &Engine, module: &Module, input: &[u8]) -> Result<Vec<u8>, WasmError> {
    let (mut store, instance) = instantiate(engine, module)?;
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or(WasmError::InvalidAbi)?;
    let allocate = instance
        .get_typed_func::<i32, i32>(&store, "pandora_alloc")
        .map_err(|_| WasmError::InvalidAbi)?;
    let run = instance
        .get_typed_func::<(i32, i32), i64>(&store, "pandora_run")
        .map_err(|_| WasmError::InvalidAbi)?;
    let input_length = i32::try_from(input.len()).map_err(|_| WasmError::InputTooLarge)?;
    let input_pointer = allocate
        .call(&mut store, input_length)
        .map_err(map_execution_error)?;
    let input_offset = usize::try_from(input_pointer).map_err(|_| WasmError::InvalidAbi)?;
    memory
        .write(&mut store, input_offset, input)
        .map_err(|_| WasmError::InvalidAbi)?;
    let packed = run
        .call(&mut store, (input_pointer, input_length))
        .map_err(map_execution_error)? as u64;
    let output_offset = (packed >> 32) as u32 as usize;
    let output_length = (packed & u64::from(u32::MAX)) as usize;
    if output_length > MAX_WASM_OUTPUT_BYTES {
        return Err(WasmError::OutputTooLarge);
    }
    output_offset
        .checked_add(output_length)
        .ok_or(WasmError::InvalidAbi)?;
    let mut output = vec![0; output_length];
    memory
        .read(&store, output_offset, &mut output)
        .map_err(|_| WasmError::InvalidAbi)?;
    serde_json::from_slice::<serde_json::Value>(&output).map_err(|_| WasmError::InvalidOutput)?;
    Ok(output)
}

fn instantiate(
    engine: &Engine,
    module: &Module,
) -> Result<(Store<StoreState>, wasmi::Instance), WasmError> {
    let limits = StoreLimitsBuilder::new()
        .memory_size(MAX_WASM_MEMORY_BYTES)
        .instances(1)
        .memories(1)
        .tables(1)
        .table_elements(1_024)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(engine, StoreState { limits });
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(DEFAULT_WASM_FUEL)
        .map_err(|_| WasmError::ResourceLimit)?;
    let linker = Linker::<StoreState>::new(engine);
    let instance = linker
        .instantiate_and_start(&mut store, module)
        .map_err(map_execution_error)?;
    Ok((store, instance))
}

fn map_execution_error(error: wasmi::Error) -> WasmError {
    if error.as_trap_code() == Some(TrapCode::OutOfFuel) {
        WasmError::ResourceLimit
    } else {
        WasmError::ExecutionFailed
    }
}

fn receipt_for(permit: &ConsumedPermit, now: Timestamp, outcome: EffectOutcome) -> EffectReceipt {
    let receipt_id = ReceiptId::new(format!(
        "receipt-wasm-{}",
        NEXT_RECEIPT_ID.fetch_add(1, Ordering::Relaxed)
    ))
    .expect("generated receipt ID is valid");
    EffectReceipt::new(
        receipt_id,
        permit.permit().permit_id().clone(),
        permit.permit().request_digest().clone(),
        now,
        outcome,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApprovalRequest, ApprovalStore, Parliament, ReferenceMonitor};
    use pandora_types::{
        ArtifactId, Capability, EffectOutcome, EffectTarget, ExecutionId, ExecutionProfile,
        ExecutionProfileBinding, ExecutionProfileBindingKind, GeneId, Operation, OperationRequest,
        PackageCompatibility, PackageKind, PackageManifest, PolicyContext, PrincipalId,
        ResourceScope, SessionId, Timestamp, TrustEvidence, hash_artifact,
    };

    fn echo_module() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "pandora_alloc") (param i32) (result i32)
                    i32.const 0)
                (func (export "pandora_run") (param i32 i32) (result i64)
                    local.get 0
                    i64.extend_i32_u
                    i64.const 32
                    i64.shl
                    local.get 1
                    i64.extend_i32_u
                    i64.or))"#,
        )
        .unwrap()
    }

    fn package(artifact: &[u8]) -> PackageManifest {
        PackageManifest::new(
            "owner/transform",
            "1.0.0",
            PackageKind::Gene,
            "owner",
            hash_artifact(artifact),
            Vec::new(),
            PackageCompatibility::new("pandora>=2.0.0").unwrap(),
            "Apache-2.0",
            TrustEvidence::unsigned(),
        )
        .unwrap()
    }

    fn profile(artifact: &[u8]) -> ExecutionProfile {
        ExecutionProfile::new(
            "2.0.0-beta.1",
            "test",
            "test",
            1,
            "workspace-1",
            hash_artifact(b"contained"),
            vec![
                ExecutionProfileBinding::new(
                    ExecutionProfileBindingKind::Executor,
                    "wasm",
                    Some("1"),
                    hash_artifact(b"wasm"),
                )
                .unwrap(),
                ExecutionProfileBinding::new(
                    ExecutionProfileBindingKind::Artifact,
                    "owner/transform",
                    Some("1.0.0"),
                    hash_artifact(artifact),
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn consumed_permit(artifact: &[u8], input: &[u8]) -> crate::ConsumedPermit {
        let manifest = package(artifact);
        let request = OperationRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            profile(artifact),
            GeneId::new("owner/transform").unwrap(),
            Some(ArtifactId::new(manifest.content_hash()).unwrap()),
            Capability::WasmExecute,
            Operation::Execute,
            EffectTarget::wasm(manifest.id().as_str(), manifest.version()),
            ResourceScope::none(),
        )
        .unwrap()
        .with_payload_digest(input)
        .unwrap();
        let policy = PolicyContext::new(1, [Capability::WasmExecute], [Operation::Execute]);
        let monitor = ReferenceMonitor::new_with_policy(policy.clone(), 60);
        let decision = Parliament::new(1).decide(&request, &policy);
        let directory = crate::test_support::new_temp_dir("pandora-wasm-approval").unwrap();
        let approvals = ApprovalStore::open(directory.join("approvals.sqlite3")).unwrap();
        approvals
            .create(
                ApprovalRequest::new(
                    "approval-wasm-1",
                    request.session_id().clone(),
                    request.execution_id().clone(),
                    request.principal_id().clone(),
                    request.gene_id().clone(),
                    request.request_digest().clone(),
                    "approve the exact Wasm invocation",
                    1,
                    Timestamp::from_unix_seconds(100),
                )
                .unwrap(),
            )
            .unwrap();
        let approver = PrincipalId::new("approver-1").unwrap();
        approvals
            .resolve(
                "approval-wasm-1",
                request.principal_id(),
                &approver,
                true,
                Timestamp::from_unix_seconds(10),
            )
            .unwrap();
        let grant = approvals
            .consume_grant(
                "approval-wasm-1",
                request.principal_id(),
                request.session_id(),
                request.execution_id(),
                request.gene_id(),
                request.request_digest(),
                Timestamp::from_unix_seconds(11),
            )
            .unwrap();
        let permit = monitor
            .authorize_after_approval_with_grant(
                request.clone(),
                decision,
                &grant,
                Timestamp::from_unix_seconds(11),
            )
            .unwrap();
        monitor
            .store()
            .consume(permit, &request, Timestamp::from_unix_seconds(12))
            .unwrap()
    }

    #[test]
    fn executes_a_fuel_bounded_import_free_gene() {
        let artifact = echo_module();
        let input = br#"{"value":1}"#;
        let permit = consumed_permit(&artifact, input);
        let mut executor = WasmExecutor::new();
        executor.register(&package(&artifact), &artifact).unwrap();

        let result = executor.execute(&permit, input, Timestamp::from_unix_seconds(12));

        assert_eq!(result.result(), Ok(&input.to_vec()));
        assert_eq!(result.receipt().outcome(), &EffectOutcome::Succeeded);
    }

    #[test]
    fn rejects_modules_that_exceed_strict_compilation_limits() {
        let mut wat = String::from("(module");
        wat.push_str(&"(func nop)".repeat(1_024));
        wat.push_str(
            r#"
                (memory (export "memory") 1)
                (func (export "pandora_alloc") (param i32) (result i32) i32.const 0)
                (func (export "pandora_run") (param i32 i32) (result i64) i64.const 0))"#,
        );
        let artifact = wat::parse_str(wat).unwrap();
        let mut executor = WasmExecutor::new();

        assert_eq!(
            executor.register(&package(&artifact), &artifact),
            Err(WasmError::InvalidModule)
        );
    }

    #[test]
    fn registration_does_not_execute_the_module_start_function() {
        let artifact = wat::parse_str(
            r#"(module
                (func $start unreachable)
                (start $start)
                (memory (export "memory") 1)
                (func (export "pandora_alloc") (param i32) (result i32) i32.const 0)
                (func (export "pandora_run") (param i32 i32) (result i64) i64.const 0))"#,
        )
        .unwrap();
        let input = br#"{}"#;
        let permit = consumed_permit(&artifact, input);
        let mut executor = WasmExecutor::new();

        executor.register(&package(&artifact), &artifact).unwrap();
        let result = executor.execute(&permit, input, Timestamp::from_unix_seconds(12));

        assert_eq!(result.result(), Err(&WasmError::ExecutionFailed));
        assert_eq!(
            result.receipt().outcome(),
            &EffectOutcome::Failed {
                code: "wasm_execution_failed".to_owned()
            }
        );
    }

    #[test]
    fn rejects_modules_with_host_imports() {
        let artifact = wat::parse_str(
            r#"(module
                (import "host" "read" (func))
                (memory (export "memory") 1)
                (func (export "pandora_alloc") (param i32) (result i32) i32.const 0)
                (func (export "pandora_run") (param i32 i32) (result i64) i64.const 0))"#,
        )
        .unwrap();
        let mut executor = WasmExecutor::new();

        assert_eq!(
            executor.register(&package(&artifact), &artifact),
            Err(WasmError::ImportsForbidden)
        );
    }

    #[test]
    fn fuel_exhaustion_fails_with_a_receipt() {
        let artifact = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "pandora_alloc") (param i32) (result i32) i32.const 0)
                (func (export "pandora_run") (param i32 i32) (result i64)
                    (loop $forever br $forever)
                    i64.const 0))"#,
        )
        .unwrap();
        let input = br#"{}"#;
        let permit = consumed_permit(&artifact, input);
        let mut executor = WasmExecutor::new();
        executor.register(&package(&artifact), &artifact).unwrap();

        let result = executor.execute(&permit, input, Timestamp::from_unix_seconds(12));

        assert_eq!(result.result(), Err(&WasmError::ResourceLimit));
        assert_eq!(
            result.receipt().outcome(),
            &EffectOutcome::Failed {
                code: "wasm_resource_limit".to_owned()
            }
        );
    }

    #[test]
    fn linear_memory_growth_stops_at_the_executor_limit() {
        let artifact = wat::parse_str(
            r#"(module
                (memory (export "memory") 1 300)
                (data (i32.const 0) "{}")
                (func (export "pandora_alloc") (param i32) (result i32) i32.const 64)
                (func (export "pandora_run") (param i32 i32) (result i64)
                    i32.const 256
                    memory.grow
                    drop
                    i64.const 2))"#,
        )
        .unwrap();
        let input = br#"{}"#;
        let permit = consumed_permit(&artifact, input);
        let mut executor = WasmExecutor::new();
        executor.register(&package(&artifact), &artifact).unwrap();

        let result = executor.execute(&permit, input, Timestamp::from_unix_seconds(12));

        assert!(result.result().is_err());
        assert!(matches!(
            result.receipt().outcome(),
            EffectOutcome::Failed { .. }
        ));
    }

    #[test]
    fn rejects_output_beyond_the_abi_limit() {
        let artifact = wat::parse_str(
            r#"(module
                (memory (export "memory") 2)
                (func (export "pandora_alloc") (param i32) (result i32) i32.const 0)
                (func (export "pandora_run") (param i32 i32) (result i64)
                    i64.const 65537))"#,
        )
        .unwrap();
        let input = br#"{}"#;
        let permit = consumed_permit(&artifact, input);
        let mut executor = WasmExecutor::new();
        executor.register(&package(&artifact), &artifact).unwrap();

        let result = executor.execute(&permit, input, Timestamp::from_unix_seconds(12));

        assert_eq!(result.result(), Err(&WasmError::OutputTooLarge));
    }

    #[test]
    fn rejects_an_artifact_substitution_before_execution() {
        let artifact = echo_module();
        let other_artifact = wat::parse_str("(module)").unwrap();
        let input = br#"{}"#;
        let permit = consumed_permit(&other_artifact, input);
        let mut executor = WasmExecutor::new();
        executor.register(&package(&artifact), &artifact).unwrap();

        let result = executor.execute(&permit, input, Timestamp::from_unix_seconds(12));

        assert_eq!(result.result(), Err(&WasmError::PermissionDenied));
    }

    #[test]
    fn package_gene_plans_the_exact_wasm_invocation() {
        let artifact = echo_module();
        let gene = WasmGene::from_package(&package(&artifact)).unwrap();
        let request = WasmGeneRequest::new(
            ExecutionId::new("execution-1").unwrap(),
            SessionId::new("session-1").unwrap(),
            PrincipalId::new("principal-1").unwrap(),
            profile(&artifact),
            r#"{"value":1}"#,
        )
        .unwrap();

        let planned = gene.plan(&request.into_gene_input().unwrap()).unwrap();

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].capability(), Capability::WasmExecute);
        assert_eq!(planned[0].operation(), Operation::Execute);
        assert_eq!(
            planned[0].target(),
            &EffectTarget::wasm("owner/transform", "1.0.0")
        );
        assert_eq!(
            planned[0].artifact_id().map(|id| id.as_str()),
            Some(package(&artifact).content_hash())
        );
        assert!(planned[0].payload_digest_matches(br#"{"value":1}"#));
    }
}
