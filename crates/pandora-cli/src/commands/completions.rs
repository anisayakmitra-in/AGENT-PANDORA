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
    $commands = if ($elements.Count -gt 1 -and $elements[1] -eq 'service') {
        'start'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'rollout') {
        'inspect'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'session') {
        'list','resume','inspect'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'job') {
        'submit','work','list','inspect','cancel','mark-interrupted'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'subagent') {
        'spawn','work','list','inspect','cancel','mark-interrupted','cleanup'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'harness') {
        'list','inspect','run'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'slash') {
        'list','resolve'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'skill') {
        'list','inspect','install','enable','disable','suspend','remove','restore'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'package') {
        'scaffold','admit','validate','sign','keygen','install','install-github','list','inspect','enable','disable','rollback','lock','verify-lock','trust-root','remove'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'registry') {
        'list','set','use','remove'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'memory') {
        'recall','audit','forget','promote','synthesize'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'approval') {
        'list','inspect','resolve'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'provider') {
        'list','set','use','test'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'mcp') {
        'list','inspect','set','remove','catalog','call'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'tool') {
        'list','inspect'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'orchestration') {
        'roles','submit','claim','complete','list','inspect','cancel','mark-interrupted','resume'
    } elseif ($elements.Count -gt 3 -and $elements[1] -eq 'strategies' -and $elements[2] -eq 'population' -and $elements[3] -eq 'list') {
        '--state'
    } elseif ($elements.Count -gt 3 -and $elements[1] -eq 'strategies' -and $elements[2] -eq 'population' -and $elements[3] -eq 'inspect') {
        '--state','--id'
    } elseif ($elements.Count -gt 2 -and $elements[1] -eq 'strategies' -and $elements[2] -eq 'population') {
        'list','inspect'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'strategies') {
        'list','population'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'evolution') {
        'generate','list','inspect','submit','evaluate','approve','stage','canary','activate','rollback'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'efficiency') {
        'rank'
    } elseif ($elements.Count -gt 2 -and $elements[1] -eq 'evaluation' -and $elements[2] -eq 'schedule') {
        'create','list','disable','claim','run','runs'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'evaluation') {
        'golden','inspect','scorecard','suite','regression','schedule'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'graph') {
        'code','knowledge','review','architecture'
    } elseif ($elements.Count -gt 2 -and $elements[1] -eq "fleet" -and $elements[2] -eq "supervisor") {
        "list","start","drain","stop","recover","heartbeat","reconcile"
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'fleet') {
        'list','register','dispatch','lease','renew','release','expire','supervisor','quarantine','revoke','kill'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'feedback') {
        'coding'
    } else {
        'help','setup','service','rollout','run','chat','tui','harness','slash','session','job','subagent','skill','package','registry','memory','approval','provider','mcp','tool','orchestration','strategies','evaluation','evolution','feedback','efficiency','fleet','graph','completions','migrate','update','uninstall','doctor'
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
    if [[ "$previous" == "service" ]]; then
        COMPREPLY=( $(compgen -W 'start' -- "$current") )
    elif [[ "$previous" == "rollout" ]]; then
        COMPREPLY=( $(compgen -W 'inspect' -- "$current") )
    elif [[ "$previous" == "session" ]]; then
        COMPREPLY=( $(compgen -W 'list resume inspect' -- "$current") )
    elif [[ "$previous" == "job" ]]; then
        COMPREPLY=( $(compgen -W 'submit work list inspect cancel mark-interrupted' -- "$current") )
    elif [[ "$previous" == "subagent" ]]; then
        COMPREPLY=( $(compgen -W 'spawn work list inspect cancel mark-interrupted cleanup' -- "$current") )
    elif [[ "$previous" == "harness" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect run' -- "$current") )
    elif [[ "$previous" == "slash" ]]; then
        COMPREPLY=( $(compgen -W 'list resolve' -- "$current") )
    elif [[ "$previous" == "skill" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect install enable disable suspend remove restore' -- "$current") )
    elif [[ "$previous" == "package" ]]; then
        COMPREPLY=( $(compgen -W 'scaffold admit validate sign keygen install install-github list inspect enable disable rollback lock verify-lock trust-root remove' -- "$current") )
    elif [[ "$previous" == "registry" ]]; then
        COMPREPLY=( $(compgen -W 'list set use remove' -- "$current") )
    elif [[ "$previous" == "memory" ]]; then
        COMPREPLY=( $(compgen -W 'recall audit forget promote synthesize consolidate provenance' -- "$current") )
    elif [[ "$previous" == "approval" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect resolve' -- "$current") )
    elif [[ "$previous" == "provider" ]]; then
        COMPREPLY=( $(compgen -W 'list set use test' -- "$current") )
    elif [[ "$previous" == "mcp" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect set remove catalog call' -- "$current") )
    elif [[ "$previous" == "tool" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect' -- "$current") )
    elif [[ "$previous" == "orchestration" ]]; then
        COMPREPLY=( $(compgen -W 'roles submit claim complete list inspect cancel mark-interrupted resume' -- "$current") )
    elif [[ "${COMP_WORDS[1]}" == "strategies" && "${COMP_WORDS[2]}" == "population" && "$previous" == "population" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect' -- "$current") )
    elif [[ "${COMP_WORDS[1]}" == "strategies" && "${COMP_WORDS[2]}" == "population" && "$previous" == "list" ]]; then
        COMPREPLY=( $(compgen -W '--state' -- "$current") )
    elif [[ "${COMP_WORDS[1]}" == "strategies" && "${COMP_WORDS[2]}" == "population" && "$previous" == "inspect" ]]; then
        COMPREPLY=( $(compgen -W '--state --id' -- "$current") )
    elif [[ "${COMP_WORDS[1]}" == "strategies" && "${COMP_WORDS[2]}" == "population" && "$previous" == "--state" ]]; then
        COMPREPLY=( $(compgen -f -- "$current") )
    elif [[ "${COMP_WORDS[1]}" == "strategies" && "${COMP_WORDS[2]}" == "population" && "$previous" == "--id" ]]; then
        COMPREPLY=()
    elif [[ "$previous" == "strategies" ]]; then
        COMPREPLY=( $(compgen -W 'list population' -- "$current") )
    elif [[ "$previous" == "efficiency" ]]; then
        COMPREPLY=( $(compgen -W 'rank' -- "$current") )
    elif [[ "${COMP_WORDS[1]}" == "evaluation" && "$previous" == "schedule" ]]; then
        COMPREPLY=( $(compgen -W 'create list disable claim run runs' -- "$current") )
    elif [[ "$previous" == "evaluation" ]]; then
        COMPREPLY=( $(compgen -W 'golden inspect scorecard suite regression schedule' -- "$current") )
    elif [[ "$previous" == "evolution" ]]; then
        COMPREPLY=( $(compgen -W 'generate list inspect submit evaluate approve stage canary activate rollback' -- "$current") )
    elif [[ "$previous" == "graph" ]]; then
        COMPREPLY=( $(compgen -W 'code knowledge review architecture' -- "$current") )
    elif [[ "$previous" == "supervisor" && "${COMP_WORDS[1]}" == "fleet" ]]; then
        COMPREPLY=( $(compgen -W 'list start drain stop recover heartbeat reconcile reap restart' -- "$current") )
    elif [[ "$previous" == "fleet" ]]; then
        COMPREPLY=( $(compgen -W 'list register dispatch lease renew release expire supervisor quarantine revoke kill' -- "$current") )
    elif [[ "$previous" == "feedback" ]]; then
        COMPREPLY=( $(compgen -W 'coding' -- "$current") )
    else
        COMPREPLY=( $(compgen -W 'help setup service rollout run chat tui harness slash session job subagent skill package registry memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor' -- "$current") )
    fi
}
complete -F _pandora_complete pandora"#
}

fn zsh() -> &'static str {
    r#"#compdef pandora
if [[ ${words[2]} == service ]]; then
    _arguments '1:command:(help setup service run chat tui harness slash session job subagent skill package registry memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:service command:(start)'
elif [[ ${words[2]} == rollout ]]; then
    _arguments '1:command:(help setup service rollout run chat tui harness slash session job subagent skill package registry memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:rollout command:(inspect)'
elif [[ ${words[2]} == session ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:session command:(list resume inspect)'
elif [[ ${words[2]} == job ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:job command:(submit work list inspect cancel mark-interrupted)'
elif [[ ${words[2]} == subagent ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:subagent command:(spawn work list inspect cancel mark-interrupted cleanup)'
elif [[ ${words[2]} == harness ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:harness command:(list inspect run)'
elif [[ ${words[2]} == slash ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:slash command:(list resolve)'
elif [[ ${words[2]} == skill ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:skill command:(list inspect install enable disable suspend remove restore)'
elif [[ ${words[2]} == package ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:package command:(scaffold admit validate sign keygen install install-github list inspect enable disable rollback lock verify-lock trust-root remove)'
elif [[ ${words[2]} == registry ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:registry command:(list set use remove)'
elif [[ ${words[2]} == memory ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry memory approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:memory command:(recall audit forget promote synthesize consolidate provenance)'
elif [[ ${words[2]} == approval ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:approval command:(list inspect resolve)'
elif [[ ${words[2]} == provider ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:provider command:(list set use test)'
elif [[ ${words[2]} == mcp ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:mcp command:(list inspect set remove catalog call)'
elif [[ ${words[2]} == tool ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:tool command:(list inspect)'
elif [[ ${words[2]} == orchestration ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:orchestration command:(roles submit claim complete list inspect cancel mark-interrupted resume)'
elif [[ ${words[2]} == strategies && ${words[3]} == population && ${words[4]} == list ]]; then
    _arguments '--state=[population state path]:path:_files'
elif [[ ${words[2]} == strategies && ${words[3]} == population && ${words[4]} == inspect ]]; then
    _arguments '--state=[population state path]:path:_files' '--id=[population ID]:population ID:'
elif [[ ${words[2]} == strategies && ${words[3]} == population ]]; then
    _arguments '3:population command:(list inspect)'
elif [[ ${words[2]} == strategies ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies efficiency fleet completions migrate update uninstall doctor)' '2:strategies command:(list population)'
elif [[ ${words[2]} == efficiency ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies evaluation evolution efficiency fleet completions migrate update uninstall doctor)' '2:efficiency command:(rank)'
elif [[ ${words[2]} == evaluation && ${words[3]} == schedule ]]; then
    _arguments '3:evaluation schedule command:(create list disable claim run runs)'
elif [[ ${words[2]} == evaluation ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies evaluation evolution efficiency fleet graph completions migrate update uninstall doctor)' '2:evaluation command:(golden inspect scorecard suite regression schedule)'
elif [[ ${words[2]} == evolution ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies evaluation evolution efficiency fleet graph completions migrate update uninstall doctor)' '2:evolution command:(generate list inspect submit evaluate approve stage canary activate rollback)'
elif [[ ${words[2]} == graph ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies evaluation evolution efficiency fleet graph completions migrate update uninstall doctor)' '2:graph command:(code knowledge review architecture)'
elif [[ ${words[2]} == fleet && ${words[3]} == supervisor ]]; then
    _arguments '3:supervisor command:(list start drain stop recover heartbeat reconcile reap restart)'
elif [[ ${words[2]} == fleet ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry approval provider mcp tool orchestration strategies evaluation evolution efficiency fleet completions migrate update uninstall doctor)' '2:fleet command:(list register dispatch lease renew release expire supervisor quarantine revoke kill)'
elif [[ ${words[2]} == feedback ]]; then
    _arguments '1:command:(help setup run chat tui harness slash session job subagent skill package registry memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:feedback command:(coding)'
else
    _arguments '1:command:(help setup service rollout run chat tui harness slash session job subagent skill package registry memory approval provider mcp tool orchestration strategies evaluation feedback efficiency fleet graph completions migrate update uninstall doctor)'
fi"#
}

fn fish() -> &'static str {
    r#"complete -c pandora -f -n '__fish_use_subcommand' -a 'help setup service rollout run chat tui harness slash session job subagent skill package registry memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor'
complete -c pandora -f -n '__fish_seen_subcommand_from harness' -a 'list inspect run'
complete -c pandora -f -n '__fish_seen_subcommand_from slash' -a 'list resolve'
complete -c pandora -f -n '__fish_seen_subcommand_from session' -a 'list resume inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from service' -a 'start'
complete -c pandora -f -n '__fish_seen_subcommand_from rollout' -a 'inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from job' -a 'submit work list inspect cancel mark-interrupted'
complete -c pandora -f -n '__fish_seen_subcommand_from subagent' -a 'spawn work list inspect cancel mark-interrupted cleanup'
complete -c pandora -f -n '__fish_seen_subcommand_from skill' -a 'list inspect install enable disable suspend remove restore'
complete -c pandora -f -n '__fish_seen_subcommand_from package' -a 'scaffold admit validate sign keygen install install-github list inspect enable disable rollback lock verify-lock trust-root remove'
complete -c pandora -f -n '__fish_seen_subcommand_from registry' -a 'list set use remove'
complete -c pandora -f -n '__fish_seen_subcommand_from memory' -a 'recall audit forget promote synthesize consolidate provenance'
complete -c pandora -f -n '__fish_seen_subcommand_from approval' -a 'list inspect resolve'
complete -c pandora -f -n '__fish_seen_subcommand_from provider' -a 'list set use test'
complete -c pandora -f -n '__fish_seen_subcommand_from mcp' -a 'list inspect set remove catalog call'
complete -c pandora -f -n '__fish_seen_subcommand_from tool' -a 'list inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from orchestration' -a 'roles submit claim complete list inspect cancel mark-interrupted resume'
complete -c pandora -f -n '__fish_seen_subcommand_from strategies; and not __fish_seen_subcommand_from population' -a 'list population'
complete -c pandora -f -n '__fish_seen_subcommand_from strategies; and __fish_seen_subcommand_from population; and not __fish_seen_subcommand_from list; and not __fish_seen_subcommand_from inspect' -a 'list inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from strategies; and __fish_seen_subcommand_from population; and __fish_seen_subcommand_from list' -l state -r
complete -c pandora -f -n '__fish_seen_subcommand_from strategies; and __fish_seen_subcommand_from population; and __fish_seen_subcommand_from inspect' -l state -r
complete -c pandora -f -n '__fish_seen_subcommand_from strategies; and __fish_seen_subcommand_from population; and __fish_seen_subcommand_from inspect' -l id -r
complete -c pandora -f -n '__fish_seen_subcommand_from efficiency' -a 'rank'
complete -c pandora -f -n '__fish_seen_subcommand_from evaluation; and not __fish_seen_subcommand_from schedule' -a 'golden inspect scorecard suite regression schedule'
complete -c pandora -f -n '__fish_seen_subcommand_from evaluation; and __fish_seen_subcommand_from schedule' -a 'create list disable claim run runs'
complete -c pandora -f -n '__fish_seen_subcommand_from evolution' -a 'generate list inspect submit evaluate approve stage canary activate rollback'
complete -c pandora -f -n '__fish_seen_subcommand_from graph' -a 'code knowledge review architecture'
complete -c pandora -f -n '__fish_seen_subcommand_from fleet; and __fish_seen_subcommand_from supervisor' -a 'list start drain stop recover heartbeat reconcile reap restart'
complete -c pandora -f -n '__fish_seen_subcommand_from fleet' -a 'list register dispatch lease renew release expire supervisor quarantine revoke kill'
complete -c pandora -f -n '__fish_seen_subcommand_from feedback' -a 'coding'"#
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
        let root_commands = "help setup service rollout run chat tui harness slash session job subagent skill package registry memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor";

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
            "'scaffold','admit','validate','sign','keygen','install','install-github','list','inspect','enable','disable','rollback','lock','verify-lock','trust-root','remove'",
            "'list','set','use','remove'",
            "'recall','audit','forget','promote','synthesize'",
            "'list','inspect','resolve'",
            "'list','set','use','test'",
            "'list','inspect','set','remove','catalog','call'",
            "'golden','inspect','scorecard','suite','regression','schedule'",
            "'create','list','disable','claim','run','runs'",
            "'list','inspect','submit','evaluate'",
            "'submit','work','list','inspect','cancel','mark-interrupted'",
            "'spawn','work','list','inspect','cancel','mark-interrupted','cleanup'",
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
                "scaffold admit validate sign keygen install install-github list inspect enable disable rollback lock verify-lock trust-root remove",
                "list set use remove",
                "recall audit forget promote synthesize",
                "list inspect resolve",
                "list set use test",
                "list inspect set remove",
                "golden inspect",
                "create list disable claim run runs",
                "list inspect submit evaluate",
                "list start drain stop recover heartbeat reconcile reap restart",
                "submit work list inspect cancel mark-interrupted",
                "spawn work list inspect cancel mark-interrupted cleanup",
                "list register dispatch lease renew release expire supervisor quarantine revoke kill",
                "roles",
                "rank",
                "list register dispatch lease renew release expire supervisor quarantine revoke kill",
            ] {
                assert!(script.contains(expected), "missing {expected} in {script}");
            }
        }
        assert!(powershell.contains("'start'"));
        assert!(bash.contains("compgen -W 'start'"));
        assert!(zsh.contains("'2:service command:(start)'"));
        assert!(fish.contains("__fish_seen_subcommand_from service' -a 'start'"));
        assert!(powershell.contains("'inspect'"));
        assert!(bash.contains("compgen -W 'inspect'"));
        assert!(zsh.contains("'2:rollout command:(inspect)'"));
        assert!(fish.contains("__fish_seen_subcommand_from rollout' -a 'inspect'"));
    }

    #[test]
    fn population_strategy_completion_exposes_read_only_commands() {
        let powershell = powershell();
        let bash = bash();
        let zsh = zsh();
        let fish = fish();

        assert!(powershell.contains("'list','population'"));
        assert!(powershell.contains("'list','inspect'"));
        assert!(powershell.contains("'--state'"));
        assert!(powershell.contains("'--state','--id'"));

        assert!(bash.contains("'list population'"));
        assert!(bash.contains("'list inspect'"));
        assert!(bash.contains("'--state'"));
        assert!(bash.contains("'--state --id'"));

        assert!(zsh.contains("'2:strategies command:(list population)'"));
        assert!(zsh.contains("'3:population command:(list inspect)'"));
        assert!(zsh.contains("'--state=[population state path]:path:_files'"));
        assert!(zsh.contains("'--id=[population ID]:population ID:'"));

        assert!(fish.contains("-a 'list population'"));
        assert!(fish.contains("-a 'list inspect'"));
        assert!(fish.contains("-l state -r"));
        assert!(fish.contains("-l id -r"));
    }
}
