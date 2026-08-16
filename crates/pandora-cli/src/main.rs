mod commands;
mod output;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let json_output = args.iter().any(|argument| argument == "--json");
    if args.first().map(String::as_str) == Some("--version") {
        output::print_success(
            output::success(
                "version",
                serde_json::json!({"pandora_version": env!("CARGO_PKG_VERSION")}),
                format!("pandora {}", env!("CARGO_PKG_VERSION")),
            ),
            json_output,
        );
        return;
    }
    match commands::execute(args) {
        Ok(result) => output::print_success(result, json_output),
        Err(error) => {
            let exit_code = error.exit_code;
            output::print_error(&error, json_output);
            std::process::exit(exit_code);
        }
    }
}
