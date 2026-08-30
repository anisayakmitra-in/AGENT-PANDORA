#!/usr/bin/env node
import { createHash } from "node:crypto";
import { cpSync, existsSync, lstatSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, relative, resolve } from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const desktopRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryRoot = resolve(desktopRoot, "..", "..");
const lifecycleSandboxPrefix = "pandora-desktop-lifecycle-";
const lifecycleSandboxes = new Set();

export function sidecarName(target) {
  return `pandora-${target}${target.includes("windows") ? ".exe" : ""}`;
}

export function packagedSidecarName(target) {
  return "pandora" + (target.includes("windows") ? ".exe" : "");
}

export function validateSidecarTarget(target) {
  const normalized = target.trim();
  if (!/^[A-Za-z0-9_.-]+$/.test(normalized)) {
    throw new Error("invalid Pandora sidecar target triple");
  }
  return normalized;
}

export function systemInstallLifecycleEnabled(environment = process.env) {
  const requested = environment.PANDORA_DESKTOP_SYSTEM_INSTALL_LIFECYCLE?.trim();
  if (!requested) return false;
  if (requested !== "1") {
    throw new Error("PANDORA_DESKTOP_SYSTEM_INSTALL_LIFECYCLE must be exactly 1");
  }
  if (environment.CI?.trim().toLowerCase() !== "true") {
    throw new Error("desktop system-install lifecycle is restricted to an ephemeral CI runner");
  }
  return true;
}

function canonicalTemporaryRoot() {
  const root = realpathSync(tmpdir());
  const metadata = lstatSync(root);
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
    throw new Error(`refusing to use a non-directory temporary root: ${tmpdir()}`);
  }
  return root;
}

export function createLifecycleSandbox() {
  const temporaryRoot = canonicalTemporaryRoot();
  const sandbox = mkdtempSync(join(temporaryRoot, lifecycleSandboxPrefix));
  const canonicalSandbox = realpathSync(sandbox);
  if (dirname(canonicalSandbox) !== temporaryRoot || !basename(canonicalSandbox).startsWith(lifecycleSandboxPrefix)) {
    throw new Error(`created lifecycle sandbox escaped the temporary root: ${sandbox}`);
  }
  lifecycleSandboxes.add(canonicalSandbox);
  return canonicalSandbox;
}

export function assertTemporarySandbox(path) {
  const sandbox = resolve(path);
  const temporaryRoot = canonicalTemporaryRoot();
  const metadata = lstatSync(sandbox);
  const canonicalSandbox = realpathSync(sandbox);
  if (
    metadata.isSymbolicLink()
    || !metadata.isDirectory()
    || sandbox !== canonicalSandbox
    || dirname(canonicalSandbox) !== temporaryRoot
    || !basename(canonicalSandbox).startsWith(lifecycleSandboxPrefix)
    || !lifecycleSandboxes.has(canonicalSandbox)
  ) {
    throw new Error(`refusing to remove a path outside the lifecycle sandbox: ${path}`);
  }
}

export function removeLifecycleSandbox(path) {
  assertTemporarySandbox(path);
  rmSync(path, { recursive: true, force: false });
  lifecycleSandboxes.delete(path);
}

export async function removeLifecycleSandboxWithRetry(path) {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    try {
      removeLifecycleSandbox(path);
      return;
    } catch (error) {
      const retryable = process.platform === "win32"
        && error instanceof Error
        && "code" in error
        && (error.code === "EPERM" || error.code === "EBUSY");
      if (!retryable || attempt === 19) throw error;
      await delay(100);
    }
  }
}

export function resolveBundleRoot(environment = process.env) {
  const configured = environment.PANDORA_DESKTOP_BUNDLE_ROOT?.trim();
  const candidate = resolve(
    configured || join(cargoTargetDirectory(), "release", "bundle"),
  );
  if (!existsSync(candidate)) {
    throw new Error(`desktop release bundle directory is missing: ${candidate}`);
  }
  const metadata = lstatSync(candidate);
  const canonical = realpathSync(candidate);
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
    throw new Error(`desktop release bundle root must be a directory and not a symlink: ${candidate}`);
  }
  return canonical;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: repositoryRoot, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} ${args.join(" ")} failed: ${result.stderr.trim()}`);
  return result.stdout.trim();
}

function cargoTargetDirectory() {
  return JSON.parse(run("cargo", ["metadata", "--format-version", "1", "--no-deps", "--locked"])).target_directory;
}

export function resolveSourceSidecar(target, requestedTarget, environment = process.env) {
  const configured = environment.PANDORA_DESKTOP_SOURCE_SIDECAR?.trim();
  const candidate = resolve(
    configured || sourceSidecar(target, requestedTarget),
  );
  if (configured && basename(candidate) !== sidecarName(target)) {
    throw new Error(`configured desktop source sidecar must be named ${sidecarName(target)}`);
  }
  if (!existsSync(candidate)) {
    throw new Error(`desktop source sidecar is missing: ${candidate}`);
  }
  const metadata = lstatSync(candidate);
  const canonical = realpathSync(candidate);
  if (metadata.isSymbolicLink() || !metadata.isFile()) {
    throw new Error(`desktop source sidecar must be a regular file and not a symlink: ${candidate}`);
  }
  return canonical;
}

export function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function walk(path) {
  const entries = [];
  for (const name of readdirSync(path)) {
    const candidate = join(path, name);
    const metadata = lstatSync(candidate);
    if (metadata.isDirectory()) entries.push(...walk(candidate));
    else entries.push(candidate);
  }
  return entries;
}

function walkDirectories(path) {
  const entries = [];
  for (const name of readdirSync(path)) {
    const candidate = join(path, name);
    const metadata = lstatSync(candidate);
    if (metadata.isDirectory()) entries.push(candidate, ...walkDirectories(candidate));
  }
  return entries;
}

function exactlyOne(candidates, description) {
  if (candidates.length !== 1) throw new Error(`expected exactly one ${description}, found ${candidates.length}`);
  return candidates[0];
}

export function findBundle(root, predicate, description) {
  return exactlyOne(walk(root).filter(predicate), description);
}

export function regularFile(path, description) {
  const metadata = lstatSync(path);
  if (metadata.isSymbolicLink() || !metadata.isFile()) throw new Error(`${description} must be a regular file, not a symlink`);
}

function delay(milliseconds) {
  return new Promise((resolveDelay) => setTimeout(resolveDelay, milliseconds));
}

function signalProcessTree(child, signal) {
  if (child.pid === undefined) return false;
  try {
    if (process.platform === "win32") child.kill(signal);
    else process.kill(-child.pid, signal);
    return true;
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ESRCH") return false;
    throw error;
  }
}

function processTreeIsAlive(child) {
  if (child.pid === undefined) return false;
  if (process.platform === "win32") return child.exitCode === null && child.signalCode === null;
  try {
    process.kill(-child.pid, 0);
    return true;
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ESRCH") return false;
    throw error;
  }
}

async function waitForProcessClose(child, milliseconds) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  await Promise.race([
    new Promise((resolveClose) => child.once("close", resolveClose)),
    delay(milliseconds),
  ]);
}

async function waitForProcessTreeExit(child, milliseconds) {
  const deadline = Date.now() + milliseconds;
  while (processTreeIsAlive(child) && Date.now() < deadline) {
    await delay(50);
  }
}

async function terminateProcessTree(child) {
  if (process.platform === "win32") {
    const result = spawnSync(
      "taskkill.exe",
      ["/PID", String(child.pid), "/T", "/F"],
      { stdio: "ignore", windowsHide: true },
    );
    if (result.error) throw result.error;
    await waitForProcessClose(child, 5_000);
    await delay(500);
    if (processTreeIsAlive(child)) {
      throw new Error("Windows desktop process tree did not stop after taskkill");
    }
    return;
  }
  signalProcessTree(child, "SIGTERM");
  await waitForProcessTreeExit(child, 5_000);
  if (processTreeIsAlive(child)) {
    signalProcessTree(child, "SIGKILL");
    await waitForProcessTreeExit(child, 5_000);
  }
  if (processTreeIsAlive(child)) throw new Error("desktop process tree did not stop after SIGKILL");
  await waitForProcessClose(child, 1_000);
}

async function boundedSmoke(command, args, environment) {
  const child = spawn(command, args, {
    cwd: environment.HOME,
    env: environment,
    stdio: "ignore",
    detached: process.platform !== "win32",
  });
  const failure = await new Promise((resolveFailure) => {
    child.once("error", resolveFailure);
    setTimeout(() => resolveFailure(null), 1_500);
  });
  if (failure) throw failure;
  if (child.exitCode !== null || child.signalCode !== null) {
    if (processTreeIsAlive(child)) await terminateProcessTree(child);
    throw new Error(`${command} exited before the bounded smoke window`);
  }
  await terminateProcessTree(child);
}

export function lifecycleEnvironment(sandbox) {
  const home = join(sandbox, "home");
  const config = join(sandbox, "config");
  const data = join(sandbox, "data");
  const workspace = join(sandbox, "workspace");
  for (const path of [home, config, data, workspace]) mkdirSync(path, { recursive: true });
  return { ...process.env, HOME: home, APPDATA: config, LOCALAPPDATA: data, XDG_CONFIG_HOME: config, XDG_DATA_HOME: data, PANDORA_CONFIG: join(config, "pandora", "config.toml"), PANDORA_DATA_DIR: join(data, "pandora"), PANDORA_WORKSPACE: workspace };
}

function sourceSidecar(target, requestedTarget) {
  const extension = target.includes("windows") ? ".exe" : "";
  return join(cargoTargetDirectory(), ...(requestedTarget ? [target] : []), "release", `pandora${extension}`);
}

function extractLinuxBundle(bundleRoot, sandbox) {
  const deb = findBundle(bundleRoot, (path) => path.endsWith(".deb"), "Linux .deb bundle");
  const installed = join(sandbox, "installed");
  mkdirSync(installed, { recursive: true });
  run("dpkg-deb", ["--extract", deb, installed]);
  return installed;
}

function copyMacBundle(
  bundleRoot,
  sandbox,
  requireDiskImage = false,
  installed = join(sandbox, "Applications", "Pandora.app"),
  selectedDiskImage,
) {
  const localApps = requireDiskImage
    ? []
    : walkDirectories(bundleRoot).filter((path) => path.endsWith("Pandora.app"));
  mkdirSync(dirname(installed), { recursive: true });
  if (localApps.length === 1) {
    cpSync(localApps[0], installed, { recursive: true, dereference: false });
    return installed;
  }
  if (localApps.length > 1) {
    throw new Error(`expected exactly one macOS app bundle, found ${localApps.length}`);
  }

  const image = selectedDiskImage
    ?? findBundle(bundleRoot, (path) => path.endsWith(".dmg"), "macOS disk image");
  const mount = join(sandbox, "mounted-disk-image");
  mkdirSync(mount);
  run("hdiutil", ["attach", "-readonly", "-nobrowse", "-quiet", "-mountpoint", mount, image]);
  try {
    const mountedApp = exactlyOne(
      walkDirectories(mount).filter((path) => path.endsWith("Pandora.app")),
      "mounted macOS app bundle",
    );
    cpSync(mountedApp, installed, { recursive: true, dereference: false });
  } finally {
    run("hdiutil", ["detach", "-quiet", mount]);
  }
  return installed;
}

function installWindowsBundle(bundleRoot, sandbox) {
  const msi = findBundle(bundleRoot, (path) => path.endsWith(".msi"), "Windows MSI bundle");
  const installed = join(sandbox, "installed");
  mkdirSync(installed, { recursive: true });
  run("msiexec.exe", ["/a", msi, "/qn", `TARGETDIR=${installed}`]);
  return installed;
}

function msiResult(argumentsList) {
  const result = spawnSync("msiexec.exe", argumentsList, {
    cwd: repositoryRoot,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  if (result.error) throw result.error;
  return result;
}

function assertMsiSuccess(result, operation, accepted = new Set([0, 3010])) {
  if (!accepted.has(result.status)) {
    throw new Error(`${operation} failed with MSI status ${result.status}: ${result.stderr.trim()}`);
  }
}

function linuxPackageIsInstalled() {
  const result = spawnSync("dpkg-query", ["--show", "--showformat=${db:Status-Abbrev}", "pandora"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  return result.status === 0 && result.stdout.trim().startsWith("ii");
}

export function systemInstallContract(bundleRoot, sandbox, platform, selectedBundle) {
  if (platform === "linux") {
    const deb = selectedBundle
      ?? findBundle(bundleRoot, (path) => path.endsWith(".deb"), "Linux .deb bundle");
    const installed = "/usr/bin";
    const binaries = [join(installed, "pandora"), join(installed, "pandora-desktop")];
    const installPackage = () => {
      run("sudo", ["dpkg", "--install", deb]);
      if (!linuxPackageIsInstalled()) throw new Error("Debian package did not register as installed");
    };
    return {
      installed,
      install() {
        if (linuxPackageIsInstalled() || binaries.some((path) => existsSync(path))) {
          throw new Error("refusing to replace a pre-existing system Pandora installation");
        }
        installPackage();
      },
      replace() {
        installPackage();
      },
      version() {
        return run("dpkg-query", ["--show", "--showformat=${Version}", "pandora"]);
      },
      uninstall(allowAnotherVersion = false) {
        const result = spawnSync("sudo", ["dpkg", "--purge", "pandora"], {
          encoding: "utf8",
          stdio: ["ignore", "pipe", "pipe"],
        });
        if (result.error) throw result.error;
        if (result.status !== 0 && linuxPackageIsInstalled()) {
          throw new Error(`Debian package purge failed: ${result.stderr.trim()}`);
        }
        if (!allowAnotherVersion && (linuxPackageIsInstalled() || binaries.some((path) => existsSync(path)))) {
          throw new Error("Debian package uninstall left Pandora installed");
        }
      },
      assertUninstalled() {
        if (linuxPackageIsInstalled() || binaries.some((path) => existsSync(path))) {
          throw new Error("Debian package uninstall left Pandora installed");
        }
      },
    };
  }

  if (platform === "darwin") {
    const installed = join(sandbox, "home", "Applications", "Pandora.app");
    const installApp = () => copyMacBundle(bundleRoot, sandbox, true, installed, selectedBundle);
    return {
      installed,
      install() {
        if (existsSync(installed)) throw new Error("refusing to replace an existing macOS Pandora app");
        installApp();
      },
      replace() {
        if (existsSync(installed)) rmSync(installed, { recursive: true, force: false });
        installApp();
      },
      version() {
        return run("plutil", [
          "-extract", "CFBundleShortVersionString", "raw", "-o", "-", join(installed, "Contents", "Info.plist"),
        ]);
      },
      uninstall() {
        if (existsSync(installed)) rmSync(installed, { recursive: true, force: false });
        if (existsSync(installed)) throw new Error("macOS app uninstall left Pandora installed");
      },
      assertUninstalled() {
        if (existsSync(installed)) throw new Error("macOS app uninstall left Pandora installed");
      },
    };
  }

  if (platform === "win32") {
    const msi = selectedBundle
      ?? findBundle(bundleRoot, (path) => path.endsWith(".msi"), "Windows MSI bundle");
    const installed = join(sandbox, "installed", "Pandora");
    const installLog = join(sandbox, "msi-install.log");
    const uninstallLog = join(sandbox, "msi-uninstall.log");
    const installPackage = () => {
      mkdirSync(dirname(installed), { recursive: true });
      const result = msiResult([
        "/i", msi, "/qn", "/norestart", `INSTALLDIR=${installed}`, "/l*v", installLog,
      ]);
      assertMsiSuccess(result, "Windows MSI install");
      if (!existsSync(installed)) throw new Error("Windows MSI did not create the configured install directory");
    };
    return {
      installed,
      install() {
        if (existsSync(installed)) throw new Error("refusing to replace an existing Windows Pandora installation");
        installPackage();
      },
      replace() {
        installPackage();
      },
      version() {
        return run("powershell.exe", [
          "-NoProfile",
          "-NonInteractive",
          "-Command",
          "(Get-Item -LiteralPath $env:PANDORA_VERSION_BINARY).VersionInfo.ProductVersion",
        ], {
          env: { ...process.env, PANDORA_VERSION_BINARY: applicationBinary(installed, platform) },
        });
      },
      uninstall(allowAnotherVersion = false) {
        const result = msiResult(["/x", msi, "/qn", "/norestart", "/l*v", uninstallLog]);
        assertMsiSuccess(result, "Windows MSI uninstall", new Set([0, 1605, 3010]));
        if (!allowAnotherVersion && existsSync(installed)) throw new Error("Windows MSI uninstall left Pandora installed");
      },
      assertUninstalled() {
        if (existsSync(installed)) throw new Error("Windows MSI uninstall left Pandora installed");
      },
    };
  }

  throw new Error(`unsupported system-install lifecycle platform: ${platform}`);
}

export function applicationBinary(installed, platform) {
  if (platform === "darwin") return join(installed, "Contents", "MacOS", "pandora-desktop");
  const extension = platform === "win32" ? ".exe" : "";
  return exactlyOne(walk(installed).filter((path) => {
    const name = basename(path).toLowerCase();
    return name === ("pandora-desktop" + extension);
  }), `${platform} desktop executable`);
}

export async function smokeInstalledBundle(installed, platform, target, environment) {
  if (platform === "darwin") {
    run("open", ["-n", installed], { env: environment });
    await delay(1_500);
    const binary = applicationBinary(installed, platform);
    regularFile(binary, "desktop executable");
    const processCheck = spawnSync("pgrep", ["-f", binary], { env: environment });
    if (processCheck.status !== 0) throw new Error("macOS app did not stay alive during the bounded smoke window");
    run("pkill", ["-f", binary], { env: environment });
    return;
  }
  const binary = applicationBinary(installed, platform);
  regularFile(binary, "desktop executable");
  if (platform === "linux") await boundedSmoke("xvfb-run", ["-a", binary], environment);
  else await boundedSmoke(binary, [], environment);
}

async function main() {
  const target = validateSidecarTarget(process.env.PANDORA_SIDECAR_TARGET ?? run("rustc", ["--print", "host-tuple"]));
  const requestedTarget = Boolean(process.env.PANDORA_SIDECAR_TARGET?.trim());
  const source = resolveSourceSidecar(target, requestedTarget);
  const systemInstall = systemInstallLifecycleEnabled();
  const sandbox = createLifecycleSandbox();
  let systemContract;
  try {
    const bundleRoot = resolveBundleRoot();
    systemContract = systemInstall
      ? systemInstallContract(bundleRoot, sandbox, process.platform)
      : undefined;
    systemContract?.install();
    const installed = systemContract?.installed
      ?? (process.platform === "linux" ? extractLinuxBundle(bundleRoot, sandbox) : process.platform === "darwin" ? copyMacBundle(bundleRoot, sandbox) : process.platform === "win32" ? installWindowsBundle(bundleRoot, sandbox) : (() => { throw new Error(`unsupported lifecycle platform: ${process.platform}`); })());
    const bundledSidecar = findBundle(installed, (path) => basename(path) === packagedSidecarName(target), "bundled sidecar");
    regularFile(bundledSidecar, "bundled sidecar");
    if (sha256(source) !== sha256(bundledSidecar)) throw new Error("bundled sidecar differs from the same-commit release binary");
    await smokeInstalledBundle(installed, process.platform, target, lifecycleEnvironment(sandbox));
    console.log(`verified ${process.platform} bundle lifecycle with ${relative(sandbox, bundledSidecar)}`);
  } finally {
    try {
      systemContract?.uninstall();
    } finally {
      await removeLifecycleSandboxWithRetry(sandbox);
    }
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
