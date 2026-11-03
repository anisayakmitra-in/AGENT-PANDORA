#![forbid(unsafe_code)]

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_E2E_FIXTURE: AtomicU64 = AtomicU64::new(1000);

pub struct E2eFixture {
    pub root: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub workspace: PathBuf,
}

impl E2eFixture {
    pub fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock available")
            .as_nanos();
        let sequence = NEXT_E2E_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let temp_root = std::env::temp_dir()
            .canonicalize()
            .expect("temp dir canonical path");
        let root = temp_root.join(format!(
            "pandora-e2e-{}-{suffix}-{sequence}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let data = root.join("data");
        let config = root.join("config.json");

        fs::create_dir_all(&workspace).expect("workspace dir created");
        fs::create_dir_all(&data).expect("data dir created");

        // Write a minimal README.md in the workspace
        fs::write(
            workspace.join("README.md"),
            "# AGENT-PANDORA E2E Workspace\n",
        )
        .expect("initial README written");

        Self {
            root,
            config,
            data,
            workspace,
        }
    }

    pub fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pandora"));
        cmd.args(args)
            .env("PANDORA_CONFIG", &self.config)
            .env("PANDORA_DATA_DIR", &self.data)
            .env("PANDORA_WORKSPACE", &self.workspace)
            .env_remove("PANDORA_PROVIDER")
            .env_remove("PANDORA_PROVIDER_URL")
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("GROQ_API_KEY")
            .env_remove("DEEPSEEK_API_KEY")
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("OLLAMA_API_KEY");
        cmd
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.command(args)
            .output()
            .unwrap_or_else(|err| panic!("failed to execute pandora with args {args:?}: {err}"))
    }

    #[allow(dead_code)]
    pub fn setup(&self) -> Value {
        let output = self.run(&[
            "setup",
            "--provider-url",
            "http://127.0.0.1:4317/v1",
            "--json",
        ]);
        assert!(
            output.status.success(),
            "setup failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        parse_json(&output)
    }

    pub fn init_git(&self) -> String {
        for args in [
            ["init"].as_slice(),
            ["config", "user.email", "pandora-e2e@example.test"].as_slice(),
            ["config", "user.name", "Pandora E2E Test"].as_slice(),
            ["add", "README.md"].as_slice(),
            ["commit", "-m", "initial commit"].as_slice(),
        ] {
            let output = Command::new("git")
                .args(args)
                .current_dir(&self.workspace)
                .output()
                .expect("git command failed to start");
            assert!(
                output.status.success(),
                "git command {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.workspace)
            .output()
            .expect("git rev-parse HEAD failed");
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .expect("HEAD commit is UTF-8")
            .trim()
            .to_owned()
    }
}

impl Drop for E2eFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[allow(dead_code)]
pub fn parse_json(output: &Output) -> Value {
    let raw = if !output.stdout.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    };
    serde_json::from_slice(raw).unwrap_or_else(|err| {
        panic!(
            "failed to parse JSON from output (err: {err}):\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[allow(dead_code)]
pub fn assert_png_valid(bytes: &[u8]) {
    const PNG_HEADER: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    assert!(
        bytes.len() >= 8,
        "PNG bytes length too short: {}",
        bytes.len()
    );
    assert_eq!(
        &bytes[0..8],
        &PNG_HEADER,
        "Invalid PNG magic signature header"
    );

    // Verify IHDR chunk exists immediately after header
    assert!(bytes.len() >= 20, "PNG bytes too short for IHDR");
    let ihdr_type = &bytes[12..16];
    assert_eq!(ihdr_type, b"IHDR", "First chunk must be IHDR");

    // Verify IEND chunk exists at end
    assert!(bytes.len() >= 12, "PNG bytes too short for IEND");
    let has_iend = bytes.windows(4).any(|w| w == b"IEND");
    assert!(has_iend, "PNG missing required IEND chunk");
}

#[allow(dead_code)]
pub fn strip_unc(path: &Path) -> String {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        s.to_string()
    }
}

#[allow(dead_code)]
pub fn find_in_path(binary: &str) -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&paths) {
            let candidate = path.join(binary);
            if candidate.is_file() {
                return Some(candidate);
            }
            #[cfg(windows)]
            {
                let with_exe = path.join(format!("{binary}.exe"));
                if with_exe.is_file() {
                    return Some(with_exe);
                }
            }
        }
    }
    None
}

pub fn project_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo_root) = manifest_dir.parent().and_then(|p| p.parent())
        && repo_root.join("Cargo.toml").is_file()
    {
        return repo_root.to_path_buf();
    }
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or(manifest_dir)
}

#[allow(dead_code)]
pub fn python_embed_path() -> PathBuf {
    let root = project_root();
    if let Some(parent) = root.parent() {
        let local = parent.join("python-embed").join("python.exe");
        if local.is_file() {
            return local;
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&paths) {
            for name in &["python3", "python", "python3.exe", "python.exe"] {
                let candidate = path.join(name);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from("python")
}

#[allow(dead_code)]
pub fn gcc_path() -> PathBuf {
    let root = project_root();
    if let Some(parent) = root.parent() {
        let local = parent.join("w64devkit").join("bin").join("gcc.exe");
        if local.is_file() {
            return local;
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&paths) {
            for name in &["gcc", "gcc.exe", "clang", "clang.exe"] {
                let candidate = path.join(name);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from("gcc")
}

#[allow(dead_code)]
pub fn gxx_path() -> PathBuf {
    let root = project_root();
    if let Some(parent) = root.parent() {
        let local = parent.join("w64devkit").join("bin").join("g++.exe");
        if local.is_file() {
            return local;
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&paths) {
            for name in &["g++", "g++.exe", "clang++", "clang++.exe"] {
                let candidate = path.join(name);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from("g++")
}
