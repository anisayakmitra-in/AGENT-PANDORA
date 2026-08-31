use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime crate is nested under the repository")
        .to_path_buf();
    let examples = [
        (
            "static-guide/static-guide.wasm",
            r#"{"kind":"guidance","message":"Inspect declared capabilities before activation.","effect":null}"#,
        ),
        (
            "bounded-read/bounded-read.wasm",
            r#"{"kind":"effect_request","capability":"filesystem.read","operation":"read","path":"README.md","max_bytes":4096}"#,
        ),
        (
            "patch-proposal/patch-proposal.wasm",
            r#"{"kind":"effect_request","capability":"filesystem.write","operation":"write","path":"gene-pack-output.txt","content":"approved through the governed path"}"#,
        ),
    ];

    for (relative, output) in examples {
        let path = repository.join("sdk/gene-pack/genes").join(relative);
        fs::create_dir_all(path.parent().expect("example artifact has a parent"))
            .expect("Gene pack artifact directory is writable");
        fs::write(&path, constant_json_module(output))
            .expect("Gene pack artifact can be generated");
        println!("generated {}", path.display());
    }
}

fn constant_json_module(output: &str) -> Vec<u8> {
    let data = output
        .as_bytes()
        .iter()
        .map(|byte| format!("\\{byte:02x}"))
        .collect::<String>();
    let output_len = output.len();
    let module = format!(
        r#"(module
            (memory (export "memory") 1)
            (data (i32.const 0) "{data}")
            (func (export "pandora_alloc") (param i32) (result i32)
                i32.const 8192)
            (func (export "pandora_run") (param i32 i32) (result i64)
                i64.const {output_len}))"#,
    );
    wat::parse_str(module).expect("generated Gene pack WAT is valid")
}
