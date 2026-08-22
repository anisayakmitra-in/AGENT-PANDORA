use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &[])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "completions requires one of 'powershell', 'bash', 'zsh', or 'fish'",
        ));
    }
    let shell = parsed.positionals[0].as_str();
    let (command, script) = match shell {
        "powershell" => ("completions powershell", powershell()),
        "bash" => ("completions bash", bash()),
        "zsh" => ("completions zsh", zsh()),
        "fish" => ("completions fish", fish()),
        _ => {
            return Err(CliError::usage(
                "unsupported shell; choose powershell, bash, zsh, or fish",
            ));
        }
    };
    Ok(success(
        command,
        json!({"shell": shell, "script": script}),
        script,
    ))
}

fn powershell() -> &'static str {
    r#"Register-ArgumentCompleter -CommandName pandora -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $elements = @($commandAst.CommandElements | ForEach-Object { $_.Extent.Text })
    $commands = if ($elements.Count -gt 1 -and $elements[1] -eq 'session') {
        'list','resume','inspect'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'harness') {
        'list','inspect','run'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'slash') {
        'list','resolve'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'skill') {
        'list','inspect','install','enable','disable','suspend','remove','restore'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'package') {
        'admit','list','inspect','remove'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'approval') {
        'list','inspect','resolve'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'provider') {
        'list','set','use','test'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'tool') {
        'list','inspect'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'orchestration') {
        'roles'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'strategies') {
        'list'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'efficiency') {
        'rank'
    } else {
        'help','setup','run','chat','tui','harness','slash','session','skill','package','approval','provider','tool','orchestration','strategies','efficiency','completions','migrate','update','uninstall','doctor'
    }
    $commands |
        Where-Object { $_ -like "$wordToComplete*" } |
        ForEach-Object { [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_) }
}"#
}

fn bash() -> &'static str {
    r#"_pandora_complete() {
    local current="${COMP_WORDS[COMP_CWORD]}"
    local previous="${COMP_WORDS[COMP_CWORD-1]}"
    if [[ "$previous" == "session" ]]; then
        COMPREPLY=( $(compgen -W 'list resume inspect' -- "$current") )
    elif [[ "$previous" == "harness" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect run' -- "$current") )
    elif [[ "$previous" == "slash" ]]; then
        COMPREPLY=( $(compgen -W 'list resolve' -- "$current") )
    elif [[ "$previous" == "skill" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect install enable disable suspend remove restore' -- "$current") )
    elif [[ "$previous" == "package" ]]; then
        COMPREPLY=( $(compgen -W 'admit list inspect remove' -- "$current") )
    elif [[ "$previous" == "approval" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect resolve' -- "$current") )
    elif [[ "$previous" == "provider" ]]; then
        COMPREPLY=( $(compgen -W 'list set use test' -- "$current") )
    elif [[ "$previous" == "tool" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect' -- "$current") )
    elif [[ "$previous" == "orchestration" ]]; then
        COMPREPLY=( $(compgen -W 'roles' -- "$current") )
    elif [[ "$previous" == "strategies" ]]; then
        COMPREPLY=( $(compgen -W 'list' -- "$current") )
    elif [[ "$previous" == "efficiency" ]]; then
        COMPREPLY=( $(compgen -W 'rank' -- "$current") )
    else
        COMPREPLY=( $(compgen -W 'help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor' -- "$current") )
    fi
}
complete -F _pandora_complete pandora"#
}

fn zsh() -> &'static str {
    r#"#compdef pandora
if [[ ${words[2]} == session ]]; then
    _arguments \
        '1:command:(help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor)' \
        '2:session command:(list resume inspect)'
elif [[ ${words[2]} == harness ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor)' '2:harness command:(list inspect run)'
elif [[ ${words[2]} == slash ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor)' '2:slash command:(list resolve)'
elif [[ ${words[2]} == skill ]]; then
    _arguments \
        '1:command:(help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor)' \
        '2:skill command:(list inspect install enable disable suspend remove restore)'
elif [[ ${words[2]} == package ]]; then
    _arguments \
        '1:command:(help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor)' \
        '2:package command:(admit list inspect remove)'
elif [[ ${words[2]} == approval ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor)' '2:approval command:(list inspect resolve)'
elif [[ ${words[2]} == provider ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor)' '2:provider command:(list set use test)'
elif [[ ${words[2]} == tool ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor)' '2:tool command:(list inspect)'
elif [[ ${words[2]} == orchestration ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor)' '2:orchestration command:(roles)'
elif [[ ${words[2]} == strategies ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor)' '2:strategies command:(list)'
elif [[ ${words[2]} == efficiency ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor)' '2:efficiency command:(rank)'
else
    _arguments \
        '1:command:(help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor)'
fi"#
}

fn fish() -> &'static str {
    r#"complete -c pandora -f -n '__fish_use_subcommand' -a 'help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor'
complete -c pandora -f -n '__fish_seen_subcommand_from harness' -a 'list inspect run'
complete -c pandora -f -n '__fish_seen_subcommand_from slash' -a 'list resolve'
complete -c pandora -f -n '__fish_seen_subcommand_from session' -a 'list resume inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from skill' -a 'list inspect install enable disable suspend remove restore'
complete -c pandora -f -n '__fish_seen_subcommand_from package' -a 'admit list inspect remove'
complete -c pandora -f -n '__fish_seen_subcommand_from approval' -a 'list inspect resolve'
complete -c pandora -f -n '__fish_seen_subcommand_from provider' -a 'list set use test'
complete -c pandora -f -n '__fish_seen_subcommand_from tool' -a 'list inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from orchestration' -a 'roles'
complete -c pandora -f -n '__fish_seen_subcommand_from strategies' -a 'list'
complete -c pandora -f -n '__fish_seen_subcommand_from efficiency' -a 'rank'"#
}

#[cfg(test)]
mod tests {
    use super::{bash, fish, powershell, zsh};

    #[test]
    fn completion_scripts_cover_the_public_command_surface() {
        let powershell = powershell();
        let bash = bash();
        let zsh = zsh();
        let fish = fish();
        let root_commands = "help setup run chat tui harness slash session skill package approval provider tool orchestration strategies efficiency completions migrate update uninstall doctor";

        assert!(powershell.contains(&root_commands.replace(' ', "','")));
        for script in [bash, zsh, fish] {
            assert!(
                script.contains(root_commands),
                "missing root commands in {script}"
            );
        }

        for expected in [
            "'list','inspect','run'",
            "'list','resolve'",
            "'admit','list','inspect','remove'",
            "'list','inspect','resolve'",
            "'list','set','use','test'",
        ] {
            assert!(
                powershell.contains(expected),
                "missing {expected} in PowerShell"
            );
        }
        for script in [bash, zsh, fish] {
            for expected in [
                "list inspect run",
                "list resolve",
                "admit list inspect remove",
                "list inspect resolve",
                "list set use test",
                "roles",
                "rank",
            ] {
                assert!(script.contains(expected), "missing {expected} in {script}");
            }
        }
    }
}
