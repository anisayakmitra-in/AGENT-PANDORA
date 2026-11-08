use super::parse_options;
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::RuntimeService;
use pandora_runtime::speculative_decoding::{RadixKvCache, SpeculativeEngine, TokenProbability};
use pandora_types::ServiceEngineSummary;
use serde_json::json;

/// Read-only view of the runtime engine inventory and hardware acceleration simulators.
pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let first = args.first().map(String::as_str);
    match first {
        Some("engines") => engines(&args[1..]),
        Some("decision") => super::decision::execute(&args[1..]),
        Some("speculative") => speculative(&args[1..]),
        Some("kv-cache") => kv_cache(&args[1..]),
        Some(unknown) if !unknown.starts_with('-') => Err(CliError::usage(format!(
            "unknown runtime command '{unknown}'; expected 'engines', 'decision', 'speculative', or 'kv-cache'"
        ))),
        _ => engines(args),
    }
}

pub(crate) fn speculative(args: &[String]) -> Result<CommandResult, CliError> {
    speculative_with_cmd(args, "runtime speculative benchmark")
}

pub(crate) fn speculative_with_cmd(
    args: &[String],
    cmd_name: &'static str,
) -> Result<CommandResult, CliError> {
    let first = args.first().map(String::as_str);
    match first {
        Some("benchmark") => speculative_benchmark_with_cmd(&args[1..], cmd_name),
        Some("kv-cache") => kv_cache_with_cmd(&args[1..], "runtime kv-cache"),
        Some(other) if !other.starts_with('-') => Err(CliError::usage(format!(
            "unknown speculative subcommand '{other}'; expected 'benchmark' or 'kv-cache'"
        ))),
        _ => speculative_benchmark_with_cmd(args, cmd_name),
    }
}

pub(crate) fn kv_cache(args: &[String]) -> Result<CommandResult, CliError> {
    kv_cache_with_cmd(args, "runtime kv-cache")
}

pub(crate) fn kv_cache_with_cmd(
    args: &[String],
    cmd_name: &'static str,
) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "sessions",
            "prefix-len",
            "suffix-len",
            "bytes-per-token",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "kv-cache does not accept positional arguments",
        ));
    }

    let sessions: usize = if let Some(s) = parsed.value("sessions") {
        s.parse()
            .map_err(|_| CliError::usage("--sessions must be a positive integer"))?
    } else {
        8
    };
    if sessions == 0 {
        return Err(CliError::usage("--sessions must be greater than 0"));
    }

    let prefix_len: usize = if let Some(p) = parsed.value("prefix-len") {
        p.parse()
            .map_err(|_| CliError::usage("--prefix-len must be an integer"))?
    } else {
        512
    };
    if prefix_len == 0 {
        return Err(CliError::usage("--prefix-len must be greater than 0"));
    }

    let suffix_len: usize = if let Some(s) = parsed.value("suffix-len") {
        s.parse()
            .map_err(|_| CliError::usage("--suffix-len must be an integer"))?
    } else {
        256
    };

    let bytes_per_token: usize = if let Some(b) = parsed.value("bytes-per-token") {
        b.parse()
            .map_err(|_| CliError::usage("--bytes-per-token must be an integer"))?
    } else {
        2048
    };
    if bytes_per_token == 0 {
        return Err(CliError::usage("--bytes-per-token must be greater than 0"));
    }

    let mut cache = RadixKvCache::new(bytes_per_token);
    let common_prefix: Vec<u32> = (1..=prefix_len as u32).collect();

    for s in 0..sessions {
        let mut tokens = common_prefix.clone();
        let suffix_start = 1_000_000 + (s * (suffix_len + 100)) as u32;
        tokens.extend(suffix_start..suffix_start + suffix_len as u32);
        cache.insert(&format!("session_{s}"), &tokens);
    }

    let tokens_per_session = prefix_len + suffix_len;
    let total_tokens_requested = sessions * tokens_per_session;
    let total_tokens_shared = (sessions - 1) * prefix_len;
    let dense_memory_bytes = total_tokens_requested * bytes_per_token;
    let compressed_memory_bytes = (prefix_len + sessions * suffix_len) * bytes_per_token;
    let dense_memory_mib = dense_memory_bytes as f64 / (1024.0 * 1024.0);
    let compressed_memory_mib = compressed_memory_bytes as f64 / (1024.0 * 1024.0);
    let prefix_fraction_eta = prefix_len as f64 / tokens_per_session as f64;

    let empirical_cr = cache.compression_ratio();
    let theoretical_cr = RadixKvCache::theoretical_compression_ratio(sessions, prefix_fraction_eta);
    let asymptotic_cr = RadixKvCache::asymptotic_compression_ratio(prefix_fraction_eta);
    let memory_savings_percent =
        (1.0 - (compressed_memory_bytes as f64 / dense_memory_bytes as f64)) * 100.0;

    let human = format!(
        "Prefix-Tree Radix KV Cache Telemetry:\n  Active Sessions (M): {sessions}\n  Common Prefix Length: {prefix_len} tokens (prefix ratio eta: {:.2}%)\n  Unique Suffix Length: {suffix_len} tokens per session\n  Memory Footprint: {bytes_per_token} bytes/token\n  Total Tokens Requested: {total_tokens_requested} tokens across {sessions} sessions\n  Shared Prefix Tokens Reused: {total_tokens_shared} tokens\n  Dense Uncompressed Memory: {:.2} MiB ({dense_memory_bytes} bytes)\n  Compressed Radix Memory: {:.2} MiB ({compressed_memory_bytes} bytes)\n  Empirical Compression Ratio C_R: {:.2}x\n  Theoretical Compression Ratio C_R(M, eta): {:.2}x\n  Asymptotic Limit (M -> inf): {:.2}x (1 / (1 - eta))\n  Net Memory Savings: {:.2}% ({:.2} MiB saved)",
        prefix_fraction_eta * 100.0,
        dense_memory_mib,
        compressed_memory_mib,
        empirical_cr,
        theoretical_cr,
        asymptotic_cr,
        memory_savings_percent,
        dense_memory_mib - compressed_memory_mib
    );

    Ok(success(
        cmd_name,
        json!({
            "active_sessions": sessions,
            "prefix_length_tokens": prefix_len,
            "suffix_length_tokens": suffix_len,
            "bytes_per_token": bytes_per_token,
            "prefix_fraction_eta": prefix_fraction_eta,
            "total_tokens_requested": total_tokens_requested,
            "total_tokens_shared": total_tokens_shared,
            "dense_memory_bytes": dense_memory_bytes,
            "dense_memory_mib": dense_memory_mib,
            "compressed_memory_bytes": compressed_memory_bytes,
            "compressed_memory_mib": compressed_memory_mib,
            "empirical_compression_ratio": empirical_cr,
            "theoretical_compression_ratio": theoretical_cr,
            "asymptotic_compression_ratio": asymptotic_cr,
            "memory_savings_percent": memory_savings_percent
        }),
        human,
    ))
}

pub(crate) fn speculative_benchmark_with_cmd(
    args: &[String],
    cmd_name: &'static str,
) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "gamma",
            "alpha",
            "latency-ratio",
            "ratio",
            "tokens",
            "seed",
            "config",
            "data-dir",
            "workspace",
            "json",
        ],
    )?;

    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "speculative benchmark does not accept positional arguments",
        ));
    }

    let gamma: usize = if let Some(g) = parsed.value("gamma") {
        g.parse()
            .map_err(|_| CliError::usage("--gamma must be an integer between 1 and 32"))?
    } else {
        5
    };
    if gamma == 0 || gamma > 32 {
        return Err(CliError::usage("--gamma must be between 1 and 32"));
    }

    let alpha: f64 = if let Some(a) = parsed.value("alpha") {
        a.parse()
            .map_err(|_| CliError::usage("--alpha must be a number between 0.0 and 1.0"))?
    } else {
        0.80
    };
    if !(0.0..=1.0).contains(&alpha) {
        return Err(CliError::usage("--alpha must be between 0.0 and 1.0"));
    }

    let latency_ratio: f64 = if let Some(r) = parsed
        .value("latency-ratio")
        .or_else(|| parsed.value("ratio"))
    {
        r.parse()
            .map_err(|_| CliError::usage("--latency-ratio must be a positive number"))?
    } else {
        0.10
    };
    if latency_ratio <= 0.0 {
        return Err(CliError::usage("--latency-ratio must be greater than 0.0"));
    }

    let target_tokens: usize = if let Some(t) = parsed.value("tokens") {
        t.parse()
            .map_err(|_| CliError::usage("--tokens must be a positive integer"))?
    } else {
        128
    };
    if target_tokens == 0 {
        return Err(CliError::usage("--tokens must be greater than 0"));
    }

    let seed: u64 = if let Some(s) = parsed.value("seed") {
        s.parse()
            .map_err(|_| CliError::usage("--seed must be a valid 64-bit integer"))?
    } else {
        0xfeed_face_cafe_beef
    };

    let mut engine = SpeculativeEngine::with_seed(seed);
    let mut total_accepted = 0usize;
    let mut total_bonus = 0usize;
    let mut total_emitted = 0usize;
    let mut total_drafted = 0usize;
    let mut cycles_count = 0usize;

    while total_emitted < target_tokens {
        let mut draft_tokens = Vec::with_capacity(gamma);
        let mut draft_dists = Vec::with_capacity(gamma);
        let mut target_dists = Vec::with_capacity(gamma + 1);

        for i in 0..gamma {
            let tok_id = 1000 + (cycles_count * 10 + i) as u32;
            let alt_id = 2000 + (cycles_count * 10 + i) as u32;

            draft_tokens.push(tok_id);
            draft_dists.push(vec![TokenProbability::new(tok_id, 1.0)]);

            if (alpha - 1.0).abs() < 1e-9 {
                target_dists.push(vec![TokenProbability::new(tok_id, 1.0)]);
            } else if alpha <= 1e-9 {
                target_dists.push(vec![TokenProbability::new(alt_id, 1.0)]);
            } else {
                target_dists.push(vec![
                    TokenProbability::new(tok_id, alpha),
                    TokenProbability::new(alt_id, 1.0 - alpha),
                ]);
            }
        }
        target_dists.push(vec![TokenProbability::new(
            3000 + (cycles_count * 10) as u32,
            1.0,
        )]);

        let result = engine.verify_draft(&draft_tokens, &draft_dists, &target_dists);
        total_accepted += result.accepted_tokens.len();
        if result.bonus_token.is_some() {
            total_bonus += 1;
        }
        total_emitted += result.total_emitted;
        total_drafted += gamma;
        cycles_count += 1;
    }

    let empirical_acceptance_rate = if total_drafted > 0 {
        total_accepted as f64 / total_drafted as f64
    } else {
        1.0
    };
    let empirical_tokens_per_cycle = total_emitted as f64 / cycles_count as f64;
    let empirical_speedup = empirical_tokens_per_cycle / (gamma as f64 * latency_ratio + 1.0);

    let theoretical_expected = SpeculativeEngine::theoretical_expected_tokens(alpha, gamma);
    let theoretical_speedup = SpeculativeEngine::theoretical_speedup(alpha, gamma, latency_ratio);
    let (optimal_gamma, optimal_speedup) =
        SpeculativeEngine::optimal_draft_length(alpha, latency_ratio, 16);

    let human = format!(
        "Speculative Decoding Acceptance Benchmark:\n  Draft Window Length (gamma): {gamma}\n  Target Acceptance Probability (alpha): {:.4}\n  Draft Latency Ratio (r): {:.4} (draft latency {:.1}% of target)\n  Total Tokens Simulated: {total_emitted} across {cycles_count} verification cycles\n  Empirical Acceptance Rate (alpha_hat): {:.2}% ({total_accepted} accepted / {total_drafted} drafted)\n  Empirical Bonus Tokens Emitted: {total_bonus}\n  Empirical Speedup: {:.2}x ({:.2} tokens/cycle)\n  Theoretical Expected Tokens E[beta]: {:.4} tokens/cycle\n  Theoretical Speedup S(alpha, gamma, r): {:.4}x\n  Optimal Draft Length gamma*: {optimal_gamma} tokens (max speedup: {:.4}x)\n  Verdict: Speculative decoding achieves {:.2}x empirical speedup over autoregressive baseline",
        alpha,
        latency_ratio,
        latency_ratio * 100.0,
        empirical_acceptance_rate * 100.0,
        empirical_speedup,
        empirical_tokens_per_cycle,
        theoretical_expected,
        theoretical_speedup,
        optimal_speedup,
        empirical_speedup
    );

    Ok(success(
        cmd_name,
        json!({
            "draft_window_gamma": gamma,
            "acceptance_probability_alpha": alpha,
            "latency_ratio": latency_ratio,
            "total_tokens_simulated": total_emitted,
            "cycles_count": cycles_count,
            "empirical_accepted_tokens": total_accepted,
            "empirical_bonus_tokens": total_bonus,
            "empirical_acceptance_rate": empirical_acceptance_rate,
            "empirical_tokens_per_cycle": empirical_tokens_per_cycle,
            "empirical_speedup": empirical_speedup,
            "theoretical_expected_tokens": theoretical_expected,
            "theoretical_speedup": theoretical_speedup,
            "optimal_draft_length": optimal_gamma,
            "optimal_theoretical_speedup": optimal_speedup,
            "seed": seed
        }),
        human,
    ))
}

fn engines(args: &[String]) -> Result<CommandResult, CliError> {
    let selected = args.iter().position(|arg| !arg.starts_with('-'));
    let (subcommand, rest) = match selected {
        Some(index) => (args[index].as_str(), &args[index + 1..]),
        None => ("list", args),
    };
    match subcommand {
        "list" => list(rest),
        "inspect" => inspect(rest),
        unknown => Err(CliError::usage(format!(
            "unknown runtime engines command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "json"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "runtime engines list does not accept positional arguments",
        ));
    }
    let inventory = RuntimeService::engine_inventory();
    let engines = inventory.iter().map(engine_value).collect::<Vec<_>>();
    let mut categories = std::collections::BTreeMap::<&str, usize>::new();
    for engine in &inventory {
        *categories.entry(engine.category()).or_default() += 1;
    }
    let summary = categories
        .iter()
        .map(|(name, count)| format!("{name} {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(success(
        "runtime engines list",
        json!({
            "engines": engines,
            "count": inventory.len(),
            "categories": categories,
        }),
        format!("{} engines ({summary})", inventory.len()),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "json"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "runtime engines inspect requires exactly one engine ID",
        ));
    }
    let engine_id = &parsed.positionals[0];
    let engine = RuntimeService::engine_inventory()
        .into_iter()
        .find(|engine| engine.id() == engine_id)
        .ok_or_else(|| CliError::usage(format!("unknown engine '{engine_id}'")))?;
    Ok(success(
        "runtime engines inspect",
        json!({"engine": engine_value(&engine)}),
        format!(
            "{} — {} [{}]",
            engine.name(),
            engine.authority(),
            engine.category()
        ),
    ))
}

fn engine_value(engine: &ServiceEngineSummary) -> serde_json::Value {
    json!({
        "id": engine.id(),
        "name": engine.name(),
        "role": engine.role(),
        "authority": engine.authority(),
        "category": engine.category(),
        "component_kind": engine.component_kind(),
        "inputs": engine.inputs(),
        "outputs": engine.outputs(),
        "invariants": engine.invariants(),
        "evidence": engine.evidence(),
        "source_modules": engine.source_modules(),
        "related_components": engine.related_components(),
        "documentation": engine.documentation(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_speculative_benchmark_defaults() {
        let result =
            speculative_with_cmd(&[], "runtime speculative benchmark").expect("should succeed");
        assert_eq!(result.command, "runtime speculative benchmark");
        assert_eq!(result.data["draft_window_gamma"], 5);
        assert!(
            (result.data["acceptance_probability_alpha"]
                .as_f64()
                .unwrap()
                - 0.80)
                .abs()
                < 1e-4
        );
        assert!((result.data["latency_ratio"].as_f64().unwrap() - 0.10).abs() < 1e-4);
        assert!(result.data["total_tokens_simulated"].as_u64().unwrap() >= 128);
        assert!(result.data["empirical_speedup"].as_f64().unwrap() > 1.5);

        let theo_expected = result.data["theoretical_expected_tokens"].as_f64().unwrap();
        assert!((theo_expected - 3.68928).abs() < 1e-4);
        let theo_speedup = result.data["theoretical_speedup"].as_f64().unwrap();
        assert!((theo_speedup - 2.45952).abs() < 1e-4);
    }

    #[test]
    fn test_runtime_speculative_benchmark_custom_options() {
        let args = vec![
            "benchmark".to_string(),
            "--gamma".to_string(),
            "4".to_string(),
            "--alpha".to_string(),
            "0.90".to_string(),
            "--latency-ratio".to_string(),
            "0.05".to_string(),
            "--tokens".to_string(),
            "64".to_string(),
            "--seed".to_string(),
            "42".to_string(),
        ];
        let result =
            speculative_with_cmd(&args, "runtime speculative benchmark").expect("should succeed");
        assert_eq!(result.data["draft_window_gamma"], 4);
        assert!(
            (result.data["acceptance_probability_alpha"]
                .as_f64()
                .unwrap()
                - 0.90)
                .abs()
                < 1e-4
        );
        assert!((result.data["latency_ratio"].as_f64().unwrap() - 0.05).abs() < 1e-4);
        assert!(result.data["total_tokens_simulated"].as_u64().unwrap() >= 64);
        assert_eq!(result.data["seed"], 42);
    }

    #[test]
    fn test_runtime_speculative_validation_errors() {
        let err1 = speculative_with_cmd(
            &["--gamma".to_string(), "0".to_string()],
            "runtime speculative benchmark",
        );
        assert!(err1.is_err());
        let err2 = speculative_with_cmd(
            &["--gamma".to_string(), "40".to_string()],
            "runtime speculative benchmark",
        );
        assert!(err2.is_err());

        let err3 = speculative_with_cmd(
            &["--alpha".to_string(), "1.5".to_string()],
            "runtime speculative benchmark",
        );
        assert!(err3.is_err());

        let err4 = speculative_with_cmd(
            &["--latency-ratio".to_string(), "-0.1".to_string()],
            "runtime speculative benchmark",
        );
        assert!(err4.is_err());

        let err5 = speculative_with_cmd(
            &["--tokens".to_string(), "0".to_string()],
            "runtime speculative benchmark",
        );
        assert!(err5.is_err());

        let err6 = speculative_with_cmd(
            &["benchmark".to_string(), "extra".to_string()],
            "runtime speculative benchmark",
        );
        assert!(err6.is_err());
    }

    #[test]
    fn test_runtime_kv_cache_defaults_and_exact_compression_identities() {
        let result = kv_cache_with_cmd(&[], "runtime kv-cache").expect("should succeed");
        assert_eq!(result.command, "runtime kv-cache");
        assert_eq!(result.data["active_sessions"], 8);
        assert_eq!(result.data["prefix_length_tokens"], 512);
        assert_eq!(result.data["suffix_length_tokens"], 256);
        assert_eq!(result.data["bytes_per_token"], 2048);
        assert_eq!(result.data["total_tokens_requested"], 6144);
        assert_eq!(result.data["total_tokens_shared"], 3584);
        assert_eq!(result.data["dense_memory_bytes"], 12582912);
        assert_eq!(result.data["compressed_memory_bytes"], 5242880);

        let emp_cr = result.data["empirical_compression_ratio"].as_f64().unwrap();
        let theo_cr = result.data["theoretical_compression_ratio"]
            .as_f64()
            .unwrap();
        let asym_cr = result.data["asymptotic_compression_ratio"]
            .as_f64()
            .unwrap();
        assert!((emp_cr - 2.40).abs() < 1e-4);
        assert!((theo_cr - 2.40).abs() < 1e-4);
        assert!((asym_cr - 3.00).abs() < 1e-4);

        let savings = result.data["memory_savings_percent"].as_f64().unwrap();
        assert!((savings - (7.0 / 12.0 * 100.0)).abs() < 1e-4);
    }

    #[test]
    fn test_runtime_kv_cache_custom_options() {
        let args = vec![
            "--sessions".to_string(),
            "10".to_string(),
            "--prefix-len".to_string(),
            "800".to_string(),
            "--suffix-len".to_string(),
            "200".to_string(),
            "--bytes-per-token".to_string(),
            "1024".to_string(),
        ];
        let result = kv_cache_with_cmd(&args, "runtime kv-cache").expect("should succeed");
        assert_eq!(result.data["active_sessions"], 10);
        assert_eq!(result.data["prefix_length_tokens"], 800);
        assert_eq!(result.data["suffix_length_tokens"], 200);
        assert_eq!(result.data["bytes_per_token"], 1024);

        let emp_cr = result.data["empirical_compression_ratio"].as_f64().unwrap();
        let theo_cr = result.data["theoretical_compression_ratio"]
            .as_f64()
            .unwrap();
        assert!((emp_cr - (1.0 / 0.28)).abs() < 1e-4);
        assert!((theo_cr - (1.0 / 0.28)).abs() < 1e-4);
    }

    #[test]
    fn test_runtime_kv_cache_validation_errors() {
        let err1 = kv_cache_with_cmd(
            &["--sessions".to_string(), "0".to_string()],
            "runtime kv-cache",
        );
        assert!(err1.is_err());
        let err2 = kv_cache_with_cmd(
            &["--prefix-len".to_string(), "0".to_string()],
            "runtime kv-cache",
        );
        assert!(err2.is_err());
        let err3 = kv_cache_with_cmd(
            &["--bytes-per-token".to_string(), "0".to_string()],
            "runtime kv-cache",
        );
        assert!(err3.is_err());
        let err4 = kv_cache_with_cmd(&["unexpected".to_string()], "runtime kv-cache");
        assert!(err4.is_err());
    }

    #[test]
    fn test_execute_group_routing() {
        let res_spec =
            execute(&["speculative".to_string()]).expect("runtime speculative should succeed");
        assert_eq!(res_spec.command, "runtime speculative benchmark");

        let res_kv = execute(&["kv-cache".to_string()]).expect("runtime kv-cache should succeed");
        assert_eq!(res_kv.command, "runtime kv-cache");
    }
}
