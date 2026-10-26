use std::path::Path;

pub fn render_welcome_banner(
    provider: Option<&str>,
    model: Option<&str>,
    workspace: &Path,
    session_id: Option<&str>,
    is_color: bool,
) -> String {
    let purple = if is_color {
        "\x1b[38;2;168;85;247m"
    } else {
        ""
    };
    let violet = if is_color {
        "\x1b[38;2;147;51;234m"
    } else {
        ""
    };
    let amber = if is_color {
        "\x1b[38;2;245;158;11m"
    } else {
        ""
    };
    let cyan = if is_color { "\x1b[38;2;6;182;212m" } else { "" };
    let green = if is_color {
        "\x1b[38;2;52;211;153m"
    } else {
        ""
    };
    let muted = if is_color {
        "\x1b[38;2;113;113;122m"
    } else {
        ""
    };
    let bold = if is_color { "\x1b[1m" } else { "" };
    let reset = if is_color { "\x1b[0m" } else { "" };

    let version = env!("CARGO_PKG_VERSION");
    let model_display = truncate_str(model.unwrap_or("unconfigured"), 20);
    let provider_display = truncate_str(provider.unwrap_or("none"), 18);
    let workspace_str = truncate_str(&workspace.display().to_string(), 34);
    let session_display = truncate_str(session_id.unwrap_or("active"), 25);

    let logo = format!(
        "{purple}{bold}\
██████╗  █████╗ ███╗   ██╗██████╗  ██████╗ ██████╗  █████╗ 
██╔══██╗██╔══██╗████╗  ██║██╔══██╗██╔═══██╗██╔══██╗██╔══██╗
██████╔╝███████║██╔██╗ ██║██║  ██║██║   ██║██████╔╝███████║
██╔═══╝ ██╔══██║██║╚██╗██║██║  ██║██║   ██║██╔══██╗██╔══██║
██║     ██║  ██║██║ ╚████║██████╔╝╚██████╔╝██║  ██║██║  ██║
╚═╝     ╚═╝  ╚═╝╚═╝  ╚═══╝╚═════╝  ╚═════╝ ╚═╝  ╚═╝╚═╝  ╚═╝{reset}"
    );

    let card = format!(
        "{violet}╭─────────────────────────── {bold}Pandora Agent v{version}{reset}{violet} · {green}constitutional{violet} · {amber}hardened-wasm{violet} ───────────────────────────╮{reset}\n\
{violet}│{reset}                                   {bold}Constitutional Subsystems & Engines{reset}                                        {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣀⡀⠀⣀⣀⠀⢀⣀⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀{reset}   {cyan}governance:{reset} shadow_council, parliament, reference_monitor                           {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⢀⣠⣴⣾⣿⣿⣇⠸⣿⣿⠇⣸⣿⣿⣷⣦⣄⡀⠀⠀⠀⠀⠀⠀{reset}   {cyan}confinement:{reset} wasmtime_jail, process_job_object, landlock                               {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⢀⣠⣴⣶⠿⠋⣩⡿⣿⡿⠻⣿⡇⢠⡄⢸⣿⠟⢿⣿⢿⣍⠙⠿⣶⣦⣄⡀⠀{reset}   {cyan}memory:{reset} l0_ring, l1_context, l2_vector, l3_summary, l4_merkle                     {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠉⠉⠁⠶⠟⠋⠀⠉⠀⢀⣈⣁⡈⢁⣈⣁⡀⠀⠉⠀⠙⠻⠶⠈⠉⠉⠀⠀{reset}   {cyan}execution:{reset} execute_code, run_command, process_manage                                  {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣴⣿⡿⠛⢁⡈⠛⢿⣿⣦⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀{reset}   {cyan}file_ops:{reset} handle_read, handle_write, handle_patch, secure_search                    {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠿⣿⣦⣤⣈⠁⢠⣴⣿⠿⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀{reset}   {cyan}planning:{reset} todo_list, plan_verify, task_decompose                                    {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠉⠻⢿⣿⣦⡉⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀{reset}   {cyan}cron:{reset} cronjob_manage, schedule_recurring, cron_lease                               {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠘⢷⣦⣈⠛⠃⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀{reset}   {cyan}mcp_bridge:{reset} stdio_client, http_sse_client, oauth_pkce                              {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢠⣴⠦⠈⠙⠿⣦⡄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀{reset}   {cyan}profiles:{reset} full, coding, minimal, none                                              {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠸⣿⣤⡈⠁⢤⣿⠇⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀{reset}                                                                                      {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠛⠷⠄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀{reset}   {bold}Active Governance & Safety Policies{reset}                                                {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣀⠑⢶⣄⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀{reset}   {green}supervised:{reset} human-in-the-loop approval for mutating effects (default)                  {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣿⠁⢰⡆⠈⡿⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀{reset}   {amber}autonomous:{reset} pre-authorized capability envelopes & effect permits                   {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠳⠈⣡⠞⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀{reset}   {purple}parliament:{reset} active veto hooks & invariant verification                             {violet}│{reset}\n\
{violet}│{reset}  {purple}⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀{reset}   {muted}audit_ledger:{reset} sqlite_wal_merkle_receipts                                           {violet}│{reset}\n\
{violet}│{reset}                                                                                                                      {violet}│{reset}\n\
{violet}│{reset}  {bold}{model_display}{reset} · {provider_display:<18}   {muted}reasoning:{reset} system1_reflex + system2_lats (mcts)                                    {violet}│{reset}\n\
{violet}│{reset}  {muted}{workspace_str:<34}{reset}   {muted}crypto:{reset} argon2id + xchacha20poly1305 vault                                         {violet}│{reset}\n\
{violet}│{reset}  {muted}Session: {session_display:<25}{reset}                                                                                 {violet}│{reset}\n\
{violet}│{reset}                                   {bold}23 tools · 14 presets · /help for commands · Ctrl+M for model picker{reset}               {violet}│{reset}\n\
{violet}╰──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯{reset}"
    );

    format!("{logo}\n\n{card}\n")
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_owned()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}
