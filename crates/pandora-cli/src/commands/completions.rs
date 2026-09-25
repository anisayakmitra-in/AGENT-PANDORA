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
    $commands = if ($elements.Count -gt 2 -and $elements[1] -eq 'backup' -and $elements[2] -eq 'lifecycle') {
        'preview','record','list','inspect'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'backup') {
        'create','inspect','restore','lifecycle'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'service') {
        'start'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'auth') {
        'enroll','list','revoke'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'secret') {
        'set','list','status','remove'
    } elseif ($elements.Count -gt 2 -and $elements[1] -eq 'evolution' -and $elements[2] -eq 'rollout') {
        'configure','score','approve','promote','pause','resume','reject','retry','rollback'
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
    } elseif ($elements.Count -gt 2 -and $elements[1] -eq 'package' -and $elements[2] -eq 'cache') {
        'list','inspect','verify','events'
    } elseif ($elements.Count -gt 2 -and $elements[1] -eq 'package' -and $elements[2] -eq 'transparency') {
        'list','inspect'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'package') {
        'scaffold','admit','admit-cached','validate','sign','keygen','discover','download','download-github','install','install-github','cache','list','inspect','enable','disable','rollback','lock','verify-lock','trust-root','transparency','remove'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'registry') {
        'list','set','use','remove'
    } elseif ($elements.Count -gt 2 -and $elements[1] -eq 'runtime' -and $elements[2] -eq 'engines') {
        'list','inspect'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'runtime') {
        'engines'
    } elseif ($elements.Count -gt 2 -and $elements[1] -eq 'memory' -and $elements[2] -eq 'schedule') {
        'create','list','disable','claim','run','runs'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'memory') {
        'recall','audit','forget','compact','promote','synthesize','consolidate','provenance','schedule'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'approval') {
        'list','inspect','resolve'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'provider') {
        'list','set','use','test'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'mcp') {
        'list','inspect','set','remove','catalog','call'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'tool') {
        'list','inspect'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'orchestration') {
        'roles','submit','claim','complete','list','inspect','cancel','mark-interrupted','reconcile-failed','resume'
    } elseif ($elements.Count -gt 3 -and $elements[1] -eq 'strategies' -and $elements[2] -eq 'population' -and $elements[3] -eq 'list') {
        '--state'
    } elseif ($elements.Count -gt 3 -and $elements[1] -eq 'strategies' -and $elements[2] -eq 'population' -and $elements[3] -eq 'inspect') {
        '--state','--id'
    } elseif ($elements.Count -gt 2 -and $elements[1] -eq 'strategies' -and $elements[2] -eq 'population') {
        'list','inspect'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'strategies') {
        'list','population'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'evolution') {
        'generate','list','inspect','submit','evaluate','approve','stage','canary','rollout','activate','rollback'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'efficiency') {
        'rank'
    } elseif ($elements.Count -gt 2 -and $elements[1] -eq 'evaluation' -and $elements[2] -eq 'regression') {
        'propose','generate','list','inspect','review'
    } elseif ($elements.Count -gt 2 -and $elements[1] -eq 'evaluation' -and $elements[2] -eq 'schedule') {
        'create','list','disable','claim','run','runs'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'evaluation') {
        'golden','inspect','scorecard','suite','regression','schedule'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'graph') {
        'code','knowledge','review','architecture'
    } elseif ($elements.Count -gt 2 -and $elements[1] -eq "fleet" -and $elements[2] -eq "supervisor") {
        "list","start","drain","stop","recover","heartbeat","reconcile","reap","restart"
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'fleet') {
        'dashboard','list','register','dispatch','lease','renew','release','expire','supervisor','quarantine','revoke','kill'
    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'feedback') {
        'coding'
    } else {
        'help','setup','backup','service','auth','secret','rollout','run','chat','tui','harness','slash','session','job','subagent','skill','package','registry','runtime','memory','approval','provider','mcp','tool','orchestration','strategies','evaluation','evolution','feedback','efficiency','fleet','graph','completions','migrate','update','uninstall','doctor'
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
    if [[ "${COMP_WORDS[1]}" == "backup" && "$previous" == "lifecycle" ]]; then
        COMPREPLY=( $(compgen -W 'preview record list inspect' -- "$current") )
    elif [[ "$previous" == "backup" ]]; then
        COMPREPLY=( $(compgen -W 'create inspect restore lifecycle' -- "$current") )
    elif [[ "$previous" == "service" ]]; then
        COMPREPLY=( $(compgen -W 'start' -- "$current") )
    elif [[ "$previous" == "auth" ]]; then
        COMPREPLY=( $(compgen -W 'enroll list revoke' -- "$current") )
    elif [[ "$previous" == "secret" ]]; then
        COMPREPLY=( $(compgen -W 'set list status remove' -- "$current") )
    elif [[ "${COMP_WORDS[1]}" == "evolution" && "$previous" == "rollout" ]]; then
        COMPREPLY=( $(compgen -W 'configure score approve promote pause resume reject retry rollback' -- "$current") )
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
    elif [[ "${COMP_WORDS[1]}" == "package" && "$previous" == "cache" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect verify events' -- "$current") )
    elif [[ "${COMP_WORDS[1]}" == "package" && "$previous" == "transparency" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect' -- "$current") )
    elif [[ "$previous" == "package" ]]; then
        COMPREPLY=( $(compgen -W 'scaffold admit admit-cached validate sign keygen discover download download-github install install-github cache list inspect enable disable rollback lock verify-lock trust-root transparency remove' -- "$current") )
    elif [[ "$previous" == "registry" ]]; then
        COMPREPLY=( $(compgen -W 'list set use remove' -- "$current") )
    elif [[ "${COMP_WORDS[2]}" == "engines" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect' -- "$current") )
    elif [[ "$previous" == "runtime" ]]; then
        COMPREPLY=( $(compgen -W 'engines' -- "$current") )
    elif [[ "${COMP_WORDS[1]}" == "memory" && "$previous" == "schedule" ]]; then
        COMPREPLY=( $(compgen -W 'create list disable claim run runs' -- "$current") )
    elif [[ "$previous" == "memory" ]]; then
        COMPREPLY=( $(compgen -W 'recall audit forget compact promote synthesize consolidate provenance schedule' -- "$current") )
    elif [[ "$previous" == "approval" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect resolve' -- "$current") )
    elif [[ "$previous" == "provider" ]]; then
        COMPREPLY=( $(compgen -W 'list set use test' -- "$current") )
    elif [[ "$previous" == "mcp" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect set remove catalog call' -- "$current") )
    elif [[ "$previous" == "tool" ]]; then
        COMPREPLY=( $(compgen -W 'list inspect' -- "$current") )
    elif [[ "$previous" == "orchestration" ]]; then
        COMPREPLY=( $(compgen -W 'roles submit claim complete list inspect cancel mark-interrupted reconcile-failed resume' -- "$current") )
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
    elif [[ "${COMP_WORDS[1]}" == "evaluation" && "${COMP_WORDS[2]}" == "regression" && "$previous" == "regression" ]]; then
        COMPREPLY=( $(compgen -W 'propose generate list inspect review' -- "$current") )
    elif [[ "${COMP_WORDS[1]}" == "evaluation" && "$previous" == "schedule" ]]; then
        COMPREPLY=( $(compgen -W 'create list disable claim run runs' -- "$current") )
    elif [[ "$previous" == "evaluation" ]]; then
        COMPREPLY=( $(compgen -W 'golden inspect scorecard suite regression schedule' -- "$current") )
    elif [[ "$previous" == "evolution" ]]; then
        COMPREPLY=( $(compgen -W 'generate list inspect submit evaluate approve stage canary rollout activate rollback' -- "$current") )
    elif [[ "$previous" == "graph" ]]; then
        COMPREPLY=( $(compgen -W 'code knowledge review architecture' -- "$current") )
    elif [[ "$previous" == "supervisor" && "${COMP_WORDS[1]}" == "fleet" ]]; then
        COMPREPLY=( $(compgen -W 'list start drain stop recover heartbeat reconcile reap restart' -- "$current") )
    elif [[ "$previous" == "fleet" ]]; then
        COMPREPLY=( $(compgen -W 'dashboard list register dispatch lease renew release expire supervisor quarantine revoke kill' -- "$current") )
    elif [[ "$previous" == "feedback" ]]; then
        COMPREPLY=( $(compgen -W 'coding' -- "$current") )
    else
        COMPREPLY=( $(compgen -W 'help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor' -- "$current") )
    fi
}
complete -F _pandora_complete pandora"#
}

fn zsh() -> &'static str {
    r#"#compdef pandora
if [[ ${words[2]} == evaluation && ${words[3]} == regression ]]; then
    _arguments '3:evaluation regression command:(propose generate list inspect review)'
elif [[ ${words[2]} == backup && ${words[3]} == lifecycle ]]; then
    _arguments '3:backup lifecycle command:(preview record list inspect)'
elif [[ ${words[2]} == backup ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:backup command:(create inspect restore lifecycle)'
elif [[ ${words[2]} == service ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:service command:(start)'
elif [[ ${words[2]} == auth ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:auth command:(enroll list revoke)'
elif [[ ${words[2]} == secret ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:secret command:(set list status remove)'
elif [[ ${words[2]} == rollout ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:rollout command:(inspect)'
elif [[ ${words[2]} == session ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:session command:(list resume inspect)'
elif [[ ${words[2]} == job ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:job command:(submit work list inspect cancel mark-interrupted)'
elif [[ ${words[2]} == subagent ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:subagent command:(spawn work list inspect cancel mark-interrupted cleanup)'
elif [[ ${words[2]} == harness ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:harness command:(list inspect run)'
elif [[ ${words[2]} == slash ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:slash command:(list resolve)'
elif [[ ${words[2]} == skill ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:skill command:(list inspect install enable disable suspend remove restore)'
elif [[ ${words[2]} == package && ${words[3]} == cache ]]; then
    _arguments '3:package cache command:(list inspect verify events)'
elif [[ ${words[2]} == package && ${words[3]} == transparency ]]; then
    _arguments '3:package transparency command:(list inspect)'
elif [[ ${words[2]} == package ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:package command:(scaffold admit admit-cached validate sign keygen discover download download-github install install-github cache list inspect enable disable rollback lock verify-lock trust-root transparency remove)'
elif [[ ${words[2]} == registry ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:registry command:(list set use remove)'
elif [[ ${words[2]} == runtime && ${words[3]} == engines ]]; then
    _arguments '3:runtime engines command:(list inspect)'
elif [[ ${words[2]} == runtime ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:runtime command:(engines)'
elif [[ ${words[2]} == memory && ${words[3]} == schedule ]]; then
    _arguments '3:memory schedule command:(create list disable claim run runs)'
elif [[ ${words[2]} == memory ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:memory command:(recall audit forget compact promote synthesize consolidate provenance schedule)'
elif [[ ${words[2]} == approval ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:approval command:(list inspect resolve)'
elif [[ ${words[2]} == provider ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:provider command:(list set use test)'
elif [[ ${words[2]} == mcp ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:mcp command:(list inspect set remove catalog call)'
elif [[ ${words[2]} == tool ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:tool command:(list inspect)'
elif [[ ${words[2]} == orchestration ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:orchestration command:(roles submit claim complete list inspect cancel mark-interrupted reconcile-failed resume)'
elif [[ ${words[2]} == strategies && ${words[3]} == population && ${words[4]} == list ]]; then
    _arguments '--state=[population state path]:path:_files'
elif [[ ${words[2]} == strategies && ${words[3]} == population && ${words[4]} == inspect ]]; then
    _arguments '--state=[population state path]:path:_files' '--id=[population ID]:population ID:'
elif [[ ${words[2]} == strategies && ${words[3]} == population ]]; then
    _arguments '3:population command:(list inspect)'
elif [[ ${words[2]} == strategies ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:strategies command:(list population)'
elif [[ ${words[2]} == efficiency ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:efficiency command:(rank)'
elif [[ ${words[2]} == evaluation && ${words[3]} == schedule ]]; then
    _arguments '3:evaluation schedule command:(create list disable claim run runs)'
elif [[ ${words[2]} == evaluation ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:evaluation command:(golden inspect scorecard suite regression schedule)'
elif [[ ${words[2]} == evolution && ${words[3]} == rollout ]]; then
    _arguments '3:evolution rollout command:(configure score approve promote pause resume reject retry rollback)'
elif [[ ${words[2]} == evolution ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:evolution command:(generate list inspect submit evaluate approve stage canary rollout activate rollback)'
elif [[ ${words[2]} == graph ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:graph command:(code knowledge review architecture)'
elif [[ ${words[2]} == fleet && ${words[3]} == supervisor ]]; then
    _arguments '3:supervisor command:(list start drain stop recover heartbeat reconcile reap restart)'
elif [[ ${words[2]} == fleet ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:fleet command:(dashboard list register dispatch lease renew release expire supervisor quarantine revoke kill)'
elif [[ ${words[2]} == feedback ]]; then
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)' '2:feedback command:(coding)'
else
    _arguments '1:command:(help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor)'
fi"#
}

fn fish() -> &'static str {
    r#"complete -c pandora -f -n '__fish_use_subcommand' -a 'help setup backup service auth secret rollout run chat tui harness slash session job subagent skill package registry runtime memory approval provider mcp tool orchestration strategies evaluation evolution feedback efficiency fleet graph completions migrate update uninstall doctor'
complete -c pandora -f -n '__fish_seen_subcommand_from backup; and not __fish_seen_subcommand_from lifecycle' -a 'create inspect restore lifecycle'
complete -c pandora -f -n '__fish_seen_subcommand_from backup; and __fish_seen_subcommand_from lifecycle' -a 'preview record list inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from harness' -a 'list inspect run'
complete -c pandora -f -n '__fish_seen_subcommand_from slash' -a 'list resolve'
complete -c pandora -f -n '__fish_seen_subcommand_from session' -a 'list resume inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from service' -a 'start'
complete -c pandora -f -n '__fish_seen_subcommand_from auth' -a 'enroll list revoke'
complete -c pandora -f -n '__fish_seen_subcommand_from secret' -a 'set list status remove'
complete -c pandora -f -n '__fish_seen_subcommand_from rollout; and not __fish_seen_subcommand_from evolution' -a 'inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from job' -a 'submit work list inspect cancel mark-interrupted'
complete -c pandora -f -n '__fish_seen_subcommand_from subagent' -a 'spawn work list inspect cancel mark-interrupted cleanup'
complete -c pandora -f -n '__fish_seen_subcommand_from skill' -a 'list inspect install enable disable suspend remove restore'
complete -c pandora -f -n '__fish_seen_subcommand_from package; and not __fish_seen_subcommand_from cache; and not __fish_seen_subcommand_from transparency' -a 'scaffold admit admit-cached validate sign keygen discover download download-github install install-github cache list inspect enable disable rollback lock verify-lock trust-root transparency remove'
complete -c pandora -f -n '__fish_seen_subcommand_from package; and __fish_seen_subcommand_from cache' -a 'list inspect verify events'
complete -c pandora -f -n '__fish_seen_subcommand_from package; and __fish_seen_subcommand_from transparency' -a 'list inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from registry' -a 'list set use remove'
complete -c pandora -f -n '__fish_seen_subcommand_from runtime; and __fish_seen_subcommand_from engines' -a 'list inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from runtime; and not __fish_seen_subcommand_from engines' -a 'engines'
complete -c pandora -f -n '__fish_seen_subcommand_from memory; and not __fish_seen_subcommand_from schedule' -a 'recall audit forget compact promote synthesize consolidate provenance schedule'
complete -c pandora -f -n '__fish_seen_subcommand_from memory; and __fish_seen_subcommand_from schedule' -a 'create list disable claim run runs'
complete -c pandora -f -n '__fish_seen_subcommand_from approval' -a 'list inspect resolve'
complete -c pandora -f -n '__fish_seen_subcommand_from provider' -a 'list set use test'
complete -c pandora -f -n '__fish_seen_subcommand_from mcp' -a 'list inspect set remove catalog call'
complete -c pandora -f -n '__fish_seen_subcommand_from tool' -a 'list inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from orchestration' -a 'roles submit claim complete list inspect cancel mark-interrupted reconcile-failed resume'
complete -c pandora -f -n '__fish_seen_subcommand_from strategies; and not __fish_seen_subcommand_from population' -a 'list population'
complete -c pandora -f -n '__fish_seen_subcommand_from strategies; and __fish_seen_subcommand_from population; and not __fish_seen_subcommand_from list; and not __fish_seen_subcommand_from inspect' -a 'list inspect'
complete -c pandora -f -n '__fish_seen_subcommand_from strategies; and __fish_seen_subcommand_from population; and __fish_seen_subcommand_from list' -l state -r
complete -c pandora -f -n '__fish_seen_subcommand_from strategies; and __fish_seen_subcommand_from population; and __fish_seen_subcommand_from inspect' -l state -r
complete -c pandora -f -n '__fish_seen_subcommand_from strategies; and __fish_seen_subcommand_from population; and __fish_seen_subcommand_from inspect' -l id -r
complete -c pandora -f -n '__fish_seen_subcommand_from efficiency' -a 'rank'
complete -c pandora -f -n '__fish_seen_subcommand_from evaluation; and not __fish_seen_subcommand_from schedule' -a 'golden inspect scorecard suite regression schedule'
complete -c pandora -f -n '__fish_seen_subcommand_from evaluation; and __fish_seen_subcommand_from regression' -a 'propose generate list inspect review'
complete -c pandora -f -n '__fish_seen_subcommand_from evaluation; and __fish_seen_subcommand_from schedule' -a 'create list disable claim run runs'
complete -c pandora -f -n '__fish_seen_subcommand_from evolution; and not __fish_seen_subcommand_from rollout' -a 'generate list inspect submit evaluate approve stage canary rollout activate rollback'
complete -c pandora -f -n '__fish_seen_subcommand_from evolution; and __fish_seen_subcommand_from rollout' -a 'configure score approve promote pause resume reject retry rollback'
complete -c pandora -f -n '__fish_seen_subcommand_from graph' -a 'code knowledge review architecture'
complete -c pandora -f -n '__fish_seen_subcommand_from fleet; and __fish_seen_subcommand_from supervisor' -a 'list start drain stop recover heartbeat reconcile reap restart'
complete -c pandora -f -n '__fish_seen_subcommand_from fleet' -a 'dashboard list register dispatch lease renew release expire supervisor quarantine revoke kill'
complete -c pandora -f -n '__fish_seen_subcommand_from feedback' -a 'coding'"#
}

#[cfg(test)]
mod tests {
    use super::{bash, fish, powershell, zsh};
    use crate::commands::{ROOT_COMMANDS, powershell_root_command_words, root_command_words};

    /// Every `1:command:(...)` root list the zsh script can offer, so the zsh
    /// branches cannot drift apart from each other or from the dispatch table.
    fn zsh_root_lists(script: &str) -> Vec<&str> {
        let mut lists = Vec::new();
        let mut rest = script;
        while let Some(start) = rest.find("'1:command:(") {
            rest = &rest[start + "'1:command:(".len()..];
            let end = rest.find(')').expect("zsh root list should be closed");
            lists.push(&rest[..end]);
            rest = &rest[end..];
        }
        assert!(
            !lists.is_empty(),
            "no zsh root command list found in the completion script"
        );
        lists
    }

    #[test]
    fn completion_scripts_cover_the_public_command_surface() {
        let powershell = powershell();
        let bash = bash();
        let zsh = zsh();
        let fish = fish();
        let root_commands = root_command_words();

        assert!(powershell.contains(&powershell_root_command_words()));
        for script in [bash, zsh, fish] {
            assert!(
                script.contains(&root_commands),
                "missing root commands in {script}"
            );
        }
        for list in zsh_root_lists(zsh) {
            assert_eq!(
                list, root_commands,
                "zsh branch offers a different root command list"
            );
        }

        for expected in [
            "'create','inspect','restore','lifecycle'",
            "'preview','record','list','inspect'",
            "'list','inspect','run'",
            "'list','resolve'",
            "'scaffold','admit','admit-cached','validate','sign','keygen','discover','download','download-github','install','install-github','cache','list','inspect','enable','disable','rollback','lock','verify-lock','trust-root','transparency','remove'",
            "'list','inspect','verify','events'",
            "'list','set','use','remove'",
            "'recall','audit','forget','compact','promote','synthesize','consolidate','provenance','schedule'",
            "'list','inspect','resolve'",
            "'list','set','use','test'",
            "'list','inspect','set','remove','catalog','call'",
            "'golden','inspect','scorecard','suite','regression','schedule'",
            "'create','list','disable','claim','run','runs'",
            "'list','inspect','submit','evaluate'",
            "'submit','work','list','inspect','cancel','mark-interrupted'",
            "'spawn','work','list','inspect','cancel','mark-interrupted','cleanup'",
            "'roles','submit','claim','complete','list','inspect','cancel','mark-interrupted','reconcile-failed','resume'",
        ] {
            assert!(
                powershell.contains(expected),
                "missing {expected} in PowerShell"
            );
        }
        for script in [bash, zsh, fish] {
            for expected in [
                "create inspect restore lifecycle",
                "preview record list inspect",
                "list inspect run",
                "list resolve",
                "scaffold admit admit-cached validate sign keygen discover download download-github install install-github cache list inspect enable disable rollback lock verify-lock trust-root transparency remove",
                "list inspect verify events",
                "list set use remove",
                "recall audit forget compact promote synthesize consolidate provenance schedule",
                "list inspect resolve",
                "list set use test",
                "list inspect set remove",
                "golden inspect",
                "propose generate list inspect review",
                "create list disable claim run runs",
                "list inspect submit evaluate",
                "list start drain stop recover heartbeat reconcile reap restart",
                "submit work list inspect cancel mark-interrupted",
                "spawn work list inspect cancel mark-interrupted cleanup",
                "roles submit claim complete list inspect cancel mark-interrupted reconcile-failed resume",
                "dashboard list register dispatch lease renew release expire supervisor quarantine revoke kill",
                "roles",
                "rank",
                "dashboard list register dispatch lease renew release expire supervisor quarantine revoke kill",
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
        assert!(fish.contains("__fish_seen_subcommand_from rollout; and not __fish_seen_subcommand_from evolution' -a 'inspect'"));
        for script in [powershell, bash, zsh, fish] {
            assert!(
                script.contains("configure")
                    && script.contains("promote")
                    && script.contains("rollback"),
                "missing governed rollout completions in {script}"
            );
        }
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

    #[test]
    fn identity_and_secret_completions_cover_their_dispatched_subcommands() {
        let powershell = powershell();
        let bash = bash();
        let zsh = zsh();
        let fish = fish();

        assert!(powershell.contains("-eq 'auth'"));
        assert!(powershell.contains("'enroll','list','revoke'"));
        assert!(powershell.contains("-eq 'secret'"));
        assert!(powershell.contains("'set','list','status','remove'"));

        assert!(bash.contains(r#"[[ "$previous" == "auth" ]]"#));
        assert!(bash.contains("compgen -W 'enroll list revoke'"));
        assert!(bash.contains(r#"[[ "$previous" == "secret" ]]"#));
        assert!(bash.contains("compgen -W 'set list status remove'"));

        assert!(zsh.contains("'2:auth command:(enroll list revoke)'"));
        assert!(zsh.contains("'2:secret command:(set list status remove)'"));

        assert!(fish.contains("__fish_seen_subcommand_from auth' -a 'enroll list revoke'"));
        assert!(fish.contains("__fish_seen_subcommand_from secret' -a 'set list status remove'"));
    }

    #[test]
    fn package_transparency_completion_covers_its_dispatched_subcommands() {
        let powershell = powershell();
        let bash = bash();
        let zsh = zsh();
        let fish = fish();

        assert!(powershell.contains("-eq 'package' -and $elements[2] -eq 'transparency'"));
        assert!(powershell.contains(
            "'list','inspect'\n    } elseif ($elements.Count -gt 1 -and $elements[1] -eq 'package')"
        ));
        assert!(powershell.contains("'trust-root','transparency','remove'"));

        assert!(bash.contains(r#"&& "$previous" == "transparency""#));
        assert!(bash.contains("compgen -W 'list inspect' -- \"$current\") )\n    elif [[ \"$previous\" == \"package\" ]]"));
        assert!(bash.contains("trust-root transparency remove"));

        assert!(zsh.contains("'3:package transparency command:(list inspect)'"));
        assert!(zsh.contains("trust-root transparency remove"));

        assert!(fish.contains(
            "__fish_seen_subcommand_from package; and __fish_seen_subcommand_from transparency' -a 'list inspect'"
        ));
        assert!(fish.contains("trust-root transparency remove"));
    }

    #[test]
    fn fleet_supervisor_completion_matches_across_every_shell() {
        let supervisor = "list start drain stop recover heartbeat reconcile reap restart";
        let powershell = powershell();
        let bash = bash();
        let zsh = zsh();
        let fish = fish();

        // `reap` and `restart` are dispatched by commands::fleet, so every shell
        // must offer all nine verbs.
        let double_quoted = supervisor
            .split(' ')
            .map(|verb| format!("\"{verb}\""))
            .collect::<Vec<_>>()
            .join(",");
        assert!(
            powershell.contains(&double_quoted),
            "missing {supervisor} in PowerShell"
        );
        assert!(bash.contains(&format!("compgen -W '{supervisor}'")));
        assert!(zsh.contains(&format!("'3:supervisor command:({supervisor})'")));
        assert!(fish.contains(&format!(
            "__fish_seen_subcommand_from supervisor' -a '{supervisor}'"
        )));
    }

    #[test]
    fn regression_candidate_generation_is_offered_by_every_shell() {
        // `generate` is dispatched by commands::evaluation and documented in
        // `usage()`, so it must be offered alongside `propose`.
        assert!(powershell().contains("'propose','generate','list','inspect','review'"));
        for script in [bash(), zsh(), fish()] {
            assert!(
                script.contains("propose generate list inspect review"),
                "missing regression candidate generation in {script}"
            );
        }
    }

    #[test]
    fn root_command_words_track_the_dispatch_table() {
        assert_eq!(ROOT_COMMANDS.len(), root_command_words().split(' ').count());
        assert_eq!(
            ROOT_COMMANDS.len(),
            powershell_root_command_words().split(',').count()
        );
        assert!(ROOT_COMMANDS.contains(&"auth"));
        assert!(ROOT_COMMANDS.contains(&"secret"));
    }
}
