use serde_json::{Map, Value, json};

pub const OUTPUT_VERSION: &str = "0.1";

#[derive(Debug)]
pub struct CliError {
    pub code: &'static str,
    pub message: String,
    pub details: Value,
    pub exit_code: i32,
}

impl CliError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            code: "usage_error",
            message: message.into(),
            details: json!({}),
            exit_code: 2,
        }
    }

    pub fn configuration(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: "configuration_error",
            message: message.into(),
            details,
            exit_code: 10,
        }
    }

    pub fn provider(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: "provider_error",
            message: message.into(),
            details,
            exit_code: 20,
        }
    }

    pub fn policy(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: "policy_denied",
            message: message.into(),
            details,
            exit_code: 30,
        }
    }

    pub fn approval(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: "approval_required",
            message: message.into(),
            details,
            exit_code: 40,
        }
    }

    pub fn execution(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: "execution_failed",
            message: message.into(),
            details,
            exit_code: 50,
        }
    }

    pub fn internal(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: "internal_error",
            message: message.into(),
            details,
            exit_code: 60,
        }
    }

    pub fn update(message: impl Into<String>, details: Value) -> Self {
        Self {
            code: "update_error",
            message: message.into(),
            details,
            exit_code: 70,
        }
    }

    pub fn envelope(&self) -> Value {
        json!({
            "version": OUTPUT_VERSION,
            "code": self.code,
            "message": self.message,
            "details": self.details,
        })
    }
}

pub struct CommandResult {
    pub command: &'static str,
    pub data: Value,
    pub human: String,
    print: bool,
}

pub fn success(command: &'static str, data: Value, human: impl Into<String>) -> CommandResult {
    CommandResult {
        command,
        data,
        human: human.into(),
        print: true,
    }
}

pub fn already_printed(command: &'static str) -> CommandResult {
    CommandResult {
        command,
        data: Value::Null,
        human: String::new(),
        print: false,
    }
}

pub fn envelope(result: CommandResult) -> Value {
    let mut object = match result.data {
        Value::Object(object) => object,
        value => {
            let mut object = Map::new();
            object.insert("data".to_owned(), value);
            object
        }
    };
    object.insert(
        "version".to_owned(),
        Value::String(OUTPUT_VERSION.to_owned()),
    );
    object.insert(
        "command".to_owned(),
        Value::String(result.command.to_owned()),
    );
    Value::Object(object)
}

pub fn print_success(result: CommandResult, json_output: bool) {
    if !result.print {
        return;
    }
    if json_output {
        println!(
            "{}",
            serde_json::to_string(&envelope(result)).expect("CLI response should serialize")
        );
    } else {
        println!("{}", result.human);
    }
}

pub fn print_error(error: &CliError, json_output: bool) {
    if json_output {
        println!(
            "{}",
            serde_json::to_string(&error.envelope()).expect("CLI error should serialize")
        );
    } else {
        eprintln!("error: {}", error.message);
    }
}
