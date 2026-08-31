use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Capability {
    #[serde(rename = "filesystem.read")]
    FilesystemRead,
    #[serde(rename = "filesystem.write")]
    FilesystemWrite,
    #[serde(rename = "process.execute")]
    ProcessExecute,
    #[serde(rename = "network.connect")]
    NetworkConnect,
    #[serde(rename = "provider.invoke")]
    ProviderInvoke,
    #[serde(rename = "mcp.invoke")]
    McpInvoke,
    #[serde(rename = "wasm.execute")]
    WasmExecute,
    #[serde(rename = "package.install")]
    PackageInstall,
}

impl Capability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FilesystemRead => "filesystem.read",
            Self::FilesystemWrite => "filesystem.write",
            Self::ProcessExecute => "process.execute",
            Self::NetworkConnect => "network.connect",
            Self::ProviderInvoke => "provider.invoke",
            Self::McpInvoke => "mcp.invoke",
            Self::WasmExecute => "wasm.execute",
            Self::PackageInstall => "package.install",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Operation {
    Read,
    Write,
    Execute,
    Connect,
    Invoke,
    Install,
}

impl Operation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Execute => "execute",
            Self::Connect => "connect",
            Self::Invoke => "invoke",
            Self::Install => "install",
        }
    }
}
