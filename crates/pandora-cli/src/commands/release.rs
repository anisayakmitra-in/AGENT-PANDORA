#![forbid(unsafe_code)]

use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_ARTIFACTS: &[(&str, &str)] = &[
    ("pandora-aarch64-apple-darwin", "aarch64-apple-darwin"),
    ("pandora-x86_64-apple-darwin", "x86_64-apple-darwin"),
    (
        "pandora-x86_64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    ),
    (
        "pandora-x86_64-pc-windows-msvc.exe",
        "x86_64-pc-windows-msvc.exe",
    ),
];

const PLACEHOLDER_DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("release requires 'manifests'"))?;

    match subcommand.as_str() {
        "manifests" => manifests(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown release subcommand '{unknown}'. Available subcommands: manifests"
        ))),
    }
}

fn is_valid_hex_digest(digest: &str) -> bool {
    digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit())
}

fn parse_checksums_file(path: &Path) -> Result<BTreeMap<String, String>, CliError> {
    let content = fs::read_to_string(path).map_err(|err| {
        CliError::execution(
            format!("failed to read checksums file '{}': {err}", path.display()),
            json!({}),
        )
    })?;

    let mut checksums = BTreeMap::new();
    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() != 2 {
            return Err(CliError::execution(
                format!(
                    "malformed line {} in checksums file: {trimmed}",
                    line_num + 1
                ),
                json!({}),
            ));
        }
        let digest = parts[0].to_ascii_lowercase();
        let filename = parts[1].trim_start_matches('*').trim();
        if !is_valid_hex_digest(&digest) {
            return Err(CliError::execution(
                format!("invalid SHA-256 digest '{digest}' on line {}", line_num + 1),
                json!({}),
            ));
        }
        let clean_filename = Path::new(filename)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(filename);
        checksums.insert(clean_filename.to_string(), digest.clone());
        if filename != clean_filename {
            checksums.insert(filename.to_string(), digest);
        }
    }
    Ok(checksums)
}

fn generate_homebrew_formula(
    version: &str,
    tag: &str,
    checksums: &BTreeMap<String, String>,
) -> String {
    let arm_mac_sha = checksums
        .get("pandora-aarch64-apple-darwin")
        .map(|s| s.as_str())
        .unwrap_or(PLACEHOLDER_DIGEST);
    let intel_mac_sha = checksums
        .get("pandora-x86_64-apple-darwin")
        .map(|s| s.as_str())
        .unwrap_or(PLACEHOLDER_DIGEST);
    let linux_sha = checksums
        .get("pandora-x86_64-unknown-linux-gnu")
        .map(|s| s.as_str())
        .unwrap_or(PLACEHOLDER_DIGEST);

    let bin_call = "#{bin}/pandora --version";
    format!(
"class Pandora < Formula
  desc \"Terminal-first agent runtime\"
  homepage \"https://github.com/anisayakmitra-in/AGENT-PANDORA\"
  version \"{version}\"
  license \"MIT\"

  on_macos do
    if Hardware::CPU.arm?
      url \"https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/{tag}/pandora-aarch64-apple-darwin\"
      sha256 \"{arm_mac_sha}\"
    else
      url \"https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/{tag}/pandora-x86_64-apple-darwin\"
      sha256 \"{intel_mac_sha}\"
    end
  end

  on_linux do
    if Hardware::CPU.intel?
      url \"https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/{tag}/pandora-x86_64-unknown-linux-gnu\"
      sha256 \"{linux_sha}\"
    end
  end

  def install
    binary_name = Hardware::CPU.arm? ? \"pandora-aarch64-apple-darwin\" : (OS.mac? ? \"pandora-x86_64-apple-darwin\" : \"pandora-x86_64-unknown-linux-gnu\")
    if File.exist?(binary_name)
      bin.install binary_name => \"pandora\"
    else
      bin.install Dir[\"pandora*\"].first => \"pandora\"
    end
  end

  test do
    assert_match \"pandora\", shell_output(\"{bin_call}\")
  end
end
"
    )
}

fn generate_scoop_manifest(
    version: &str,
    tag: &str,
    checksums: &BTreeMap<String, String>,
) -> serde_json::Value {
    let win_sha = checksums
        .get("pandora-x86_64-pc-windows-msvc.exe")
        .map(|s| s.as_str())
        .unwrap_or(PLACEHOLDER_DIGEST);

    json!({
        "version": version,
        "description": "Terminal-first agent runtime",
        "homepage": "https://github.com/anisayakmitra-in/AGENT-PANDORA",
        "license": "MIT",
        "depends": "git",
        "architecture": {
            "64bit": {
                "url": format!("https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/{tag}/pandora-x86_64-pc-windows-msvc.exe"),
                "hash": win_sha,
                "bin": [
                    [
                        "pandora-x86_64-pc-windows-msvc.exe",
                        "pandora.exe"
                    ]
                ]
            }
        },
        "checkver": "https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/latest/download/checksums.txt",
        "autoupdate": {
            "architecture": {
                "64bit": {
                    "url": format!("https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/{tag}/pandora-x86_64-pc-windows-msvc.exe")
                }
            }
        }
    })
}

fn generate_winget_version_manifest(version: &str) -> String {
    format!(
        "# yaml-language-server: $schema=https://aka.ms/winget-manifest.version.1.6.0.schema.json
PackageIdentifier: AnisayakmitraIn.AGENT-PANDORA
PackageVersion: {version}
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
"
    )
}

fn generate_winget_installer_manifest(
    version: &str,
    tag: &str,
    checksums: &BTreeMap<String, String>,
) -> String {
    let win_sha = checksums
        .get("pandora-x86_64-pc-windows-msvc.exe")
        .map(|s| s.as_str())
        .unwrap_or(PLACEHOLDER_DIGEST);

    format!(
"# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.1.6.0.schema.json
PackageIdentifier: AnisayakmitraIn.AGENT-PANDORA
PackageVersion: {version}
Platform:
  - Windows.Desktop
InstallModes:
  - silent
  - interactive
Installers:
  - Architecture: x64
    InstallerType: portable
    InstallerUrl: https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/{tag}/pandora-x86_64-pc-windows-msvc.exe
    InstallerSha256: {win_sha}
    Commands:
      - pandora
ManifestType: installer
ManifestVersion: 1.6.0
"
    )
}

fn generate_winget_default_locale_manifest(version: &str) -> String {
    format!(
"# yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json
PackageIdentifier: AnisayakmitraIn.AGENT-PANDORA
PackageVersion: {version}
PackageLocale: en-US
Publisher: ANISAYAK MITRA
PackageName: AGENT-PANDORA
License: MIT
ShortDescription: Terminal-first agent runtime
Description: High-performance terminal-first autonomous agent platform and runtime.
Moniker: pandora
Tags:
  - agent
  - ai
  - cli
  - runtime
ManifestType: defaultLocale
ManifestVersion: 1.6.0
"
    )
}

fn validate_manifests(output_dir: &Path, version: &str) -> Result<(), CliError> {
    let formula_file = output_dir.join("Formula").join("pandora.rb");
    if !formula_file.is_file() {
        return Err(CliError::execution(
            format!("missing Homebrew formula: {}", formula_file.display()),
            json!({}),
        ));
    }
    let formula_content = fs::read_to_string(&formula_file)
        .map_err(|err| CliError::execution(format!("failed to read formula: {err}"), json!({})))?;
    if !formula_content.contains(&format!("version \"{version}\"")) {
        return Err(CliError::execution(
            format!("formula does not contain expected version '{version}'"),
            json!({}),
        ));
    }

    let scoop_file = output_dir.join("bucket").join("pandora.json");
    if !scoop_file.is_file() {
        return Err(CliError::execution(
            format!("missing Scoop manifest: {}", scoop_file.display()),
            json!({}),
        ));
    }
    let scoop_content = fs::read_to_string(&scoop_file).map_err(|err| {
        CliError::execution(format!("failed to read scoop manifest: {err}"), json!({}))
    })?;
    let scoop_val: serde_json::Value = serde_json::from_str(&scoop_content).map_err(|err| {
        CliError::execution(format!("invalid JSON in scoop manifest: {err}"), json!({}))
    })?;
    if scoop_val["version"] != version {
        return Err(CliError::execution(
            format!("scoop manifest version mismatch: expected '{version}'"),
            json!({}),
        ));
    }

    let winget_ver_file = output_dir.join("winget").join("manifest.version.yaml");
    let winget_inst_file = output_dir.join("winget").join("manifest.installer.yaml");
    let winget_loc_file = output_dir
        .join("winget")
        .join("manifest.defaultLocale.yaml");

    for (f, name) in [
        (&winget_ver_file, "winget version manifest"),
        (&winget_inst_file, "winget installer manifest"),
        (&winget_loc_file, "winget defaultLocale manifest"),
    ] {
        if !f.is_file() {
            return Err(CliError::execution(
                format!("missing {name}: {}", f.display()),
                json!({}),
            ));
        }
        let content = fs::read_to_string(f).map_err(|err| {
            CliError::execution(format!("failed to read {name}: {err}"), json!({}))
        })?;
        if !content.contains(&format!("PackageVersion: {version}")) {
            return Err(CliError::execution(
                format!("{name} does not contain PackageVersion: {version}"),
                json!({}),
            ));
        }
    }

    Ok(())
}

fn manifests(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["output", "version", "checksums", "validate"])?;

    let output_dir = parsed
        .value("output")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("dist/manifests"));

    let version_raw = parsed
        .value("version")
        .map(|s| s.to_string())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    let (clean_version, tag) = if let Some(stripped) = version_raw.strip_prefix('v') {
        (stripped.to_string(), version_raw.clone())
    } else {
        (version_raw.clone(), format!("v{version_raw}"))
    };

    if semver::Version::parse(&clean_version).is_err() {
        return Err(CliError::usage(format!(
            "invalid semantic version format: '{version_raw}'"
        )));
    }

    let checksums = if let Some(checksum_path) = parsed.value("checksums") {
        parse_checksums_file(Path::new(checksum_path))?
    } else if Path::new("checksums.txt").is_file() {
        parse_checksums_file(Path::new("checksums.txt"))?
    } else if Path::new("dist/checksums.txt").is_file() {
        parse_checksums_file(Path::new("dist/checksums.txt"))?
    } else {
        let mut map = BTreeMap::new();
        for (artifact, _) in DEFAULT_ARTIFACTS {
            map.insert(artifact.to_string(), PLACEHOLDER_DIGEST.to_string());
        }
        map
    };

    let formula_dir = output_dir.join("Formula");
    fs::create_dir_all(&formula_dir).map_err(|err| {
        CliError::execution(
            format!(
                "failed to create directory '{}': {err}",
                formula_dir.display()
            ),
            json!({}),
        )
    })?;
    let formula_content = generate_homebrew_formula(&clean_version, &tag, &checksums);
    let formula_path = formula_dir.join("pandora.rb");
    fs::write(&formula_path, &formula_content).map_err(|err| {
        CliError::execution(
            format!("failed to write '{}': {err}", formula_path.display()),
            json!({}),
        )
    })?;
    let _ = fs::write(output_dir.join("pandora.rb"), &formula_content);

    let bucket_dir = output_dir.join("bucket");
    fs::create_dir_all(&bucket_dir).map_err(|err| {
        CliError::execution(
            format!(
                "failed to create directory '{}': {err}",
                bucket_dir.display()
            ),
            json!({}),
        )
    })?;
    let scoop_val = generate_scoop_manifest(&clean_version, &tag, &checksums);
    let scoop_content = serde_json::to_string_pretty(&scoop_val).map_err(|err| {
        CliError::execution(format!("failed to serialize scoop JSON: {err}"), json!({}))
    })? + "\n";
    let scoop_path = bucket_dir.join("pandora.json");
    fs::write(&scoop_path, &scoop_content).map_err(|err| {
        CliError::execution(
            format!("failed to write '{}': {err}", scoop_path.display()),
            json!({}),
        )
    })?;
    let _ = fs::write(output_dir.join("pandora.json"), &scoop_content);

    let winget_dir = output_dir.join("winget");
    fs::create_dir_all(&winget_dir).map_err(|err| {
        CliError::execution(
            format!(
                "failed to create directory '{}': {err}",
                winget_dir.display()
            ),
            json!({}),
        )
    })?;
    let winget_ver_content = generate_winget_version_manifest(&clean_version);
    let winget_inst_content = generate_winget_installer_manifest(&clean_version, &tag, &checksums);
    let winget_loc_content = generate_winget_default_locale_manifest(&clean_version);

    let ver_path = winget_dir.join("manifest.version.yaml");
    let inst_path = winget_dir.join("manifest.installer.yaml");
    let loc_path = winget_dir.join("manifest.defaultLocale.yaml");

    fs::write(&ver_path, &winget_ver_content).map_err(|err| {
        CliError::execution(
            format!("failed to write '{}': {err}", ver_path.display()),
            json!({}),
        )
    })?;
    fs::write(&inst_path, &winget_inst_content).map_err(|err| {
        CliError::execution(
            format!("failed to write '{}': {err}", inst_path.display()),
            json!({}),
        )
    })?;
    fs::write(&loc_path, &winget_loc_content).map_err(|err| {
        CliError::execution(
            format!("failed to write '{}': {err}", loc_path.display()),
            json!({}),
        )
    })?;

    let _ = fs::write(
        output_dir.join("manifest.version.yaml"),
        &winget_ver_content,
    );
    let _ = fs::write(
        output_dir.join("manifest.installer.yaml"),
        &winget_inst_content,
    );
    let _ = fs::write(
        output_dir.join("manifest.defaultLocale.yaml"),
        &winget_loc_content,
    );

    validate_manifests(&output_dir, &clean_version)?;

    let human = format!(
        "Release manifests successfully generated and validated for {} ({}):\n  - Homebrew: {}\n  - Scoop: {}\n  - Winget: {}\n",
        tag.as_str(),
        clean_version,
        formula_path.display(),
        scoop_path.display(),
        winget_dir.display()
    );

    Ok(success(
        "release manifests",
        json!({
            "version": clean_version,
            "tag": tag,
            "output_dir": output_dir.display().to_string(),
            "formula": formula_path.display().to_string(),
            "scoop": scoop_path.display().to_string(),
            "winget_version": ver_path.display().to_string(),
            "winget_installer": inst_path.display().to_string(),
            "winget_default_locale": loc_path.display().to_string(),
            "status": "valid"
        }),
        human,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_release_invocation_fails_with_usage() {
        match execute(&[]) {
            Err(err) => assert!(err.message.contains("release requires 'manifests'")),
            Ok(_) => panic!("bare release should fail"),
        }
    }

    #[test]
    fn unknown_subcommand_fails() {
        match execute(&["unknown".to_string()]) {
            Err(err) => assert!(err.message.contains("unknown release subcommand 'unknown'")),
            Ok(_) => panic!("unknown subcommand should fail"),
        }
    }

    #[test]
    fn manifests_generation_and_validation() {
        let temp_dir =
            std::env::temp_dir().join(format!("pandora_test_manifests_{}", std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);

        let args = vec![
            "manifests".to_string(),
            "--output".to_string(),
            temp_dir.to_str().unwrap().to_string(),
            "--version".to_string(),
            "3.0.6".to_string(),
        ];
        let result = execute(&args).expect("manifest generation should succeed");
        assert!(
            result
                .human
                .contains("Release manifests successfully generated")
        );

        assert!(temp_dir.join("Formula").join("pandora.rb").is_file());
        assert!(temp_dir.join("bucket").join("pandora.json").is_file());
        assert!(
            temp_dir
                .join("winget")
                .join("manifest.version.yaml")
                .is_file()
        );
        assert!(
            temp_dir
                .join("winget")
                .join("manifest.installer.yaml")
                .is_file()
        );
        assert!(
            temp_dir
                .join("winget")
                .join("manifest.defaultLocale.yaml")
                .is_file()
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn manifests_invalid_version_rejected() {
        let temp_dir =
            std::env::temp_dir().join(format!("pandora_test_manifests_inv_{}", std::process::id()));
        let args = vec![
            "manifests".to_string(),
            "--output".to_string(),
            temp_dir.to_str().unwrap().to_string(),
            "--version".to_string(),
            "not-a-semver".to_string(),
        ];
        match execute(&args) {
            Err(err) => assert!(err.message.contains("invalid semantic version format")),
            Ok(_) => panic!("invalid version should fail"),
        }
    }
}
