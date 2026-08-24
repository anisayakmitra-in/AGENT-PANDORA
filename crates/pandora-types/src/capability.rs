#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Capability {
    FilesystemRead,
    FilesystemWrite,
    ProcessExecute,
    NetworkConnect,
    ProviderInvoke,
    McpInvoke,
    WasmExecute,
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
