mod commands;
mod output;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let json_output = args.iter().any(|argument| argument == "--json");
    let non_json_args = args
        .iter()
        .filter(|argument| argument.as_str() != "--json")
        .collect::<Vec<_>>();
    if non_json_args.len() == 1 && non_json_args[0] == "--version" {
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
