mod commands;
mod output;

fn main() {
    let recorder = pandora_runtime::config::RuntimeConfig::load(
        pandora_runtime::config::ConfigOverrides::default(),
    )
    .ok()
    .map(|config| {
        pandora_runtime::OperationalRecorder::new(
            config.data_dir(),
            "pandora-cli",
            env!("CARGO_PKG_VERSION"),
        )
    });
    if let Some(recorder) = &recorder {
        recorder.install_crash_reporter();
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let uninstalling = args.first().is_some_and(|command| command == "uninstall");
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
        if let Some(recorder) = &recorder {
            recorder.record(
                pandora_runtime::OperationalEvent::CliInvocation,
                pandora_runtime::OperationalStatus::Succeeded,
            );
        }
        return;
    }
    match commands::execute(args) {
        Ok(result) => {
            output::print_success(result, json_output);
            if let Some(recorder) = &recorder
                && !uninstalling
            {
                recorder.record(
                    pandora_runtime::OperationalEvent::CliInvocation,
                    pandora_runtime::OperationalStatus::Succeeded,
                );
            }
        }
        Err(error) => {
            let exit_code = error.exit_code;
            output::print_error(&error, json_output);
            if let Some(recorder) = &recorder
                && !uninstalling
            {
                recorder.record(
                    pandora_runtime::OperationalEvent::CliInvocation,
                    pandora_runtime::OperationalStatus::Failed,
                );
            }
            std::process::exit(exit_code);
        }
    }
}
