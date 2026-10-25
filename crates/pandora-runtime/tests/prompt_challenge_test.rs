#![forbid(unsafe_code)]

use pandora_runtime::prompt_engine::{
    PresetKind, PromptEngine, PromptError, PromptPreset, StorageTier, validate_preset_name,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock available")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), nanos));
    fs::create_dir_all(&dir).expect("temp dir created");
    dir
}

// ---------------------------------------------------------------------------
// 1. Parameter Interpolation Edge Cases
// ---------------------------------------------------------------------------

#[test]
fn test_interpolation_missing_variable_returns_specific_error() {
    let mut params = BTreeMap::new();
    params.insert("workspace".to_owned(), "/tmp".to_owned());
    let template = "Run in {{workspace}} with {{missing_var}} on {{another_missing}}.";
    let err = PromptEngine::interpolate(template, &params).unwrap_err();
    match err {
        PromptError::MissingParameter { param, template: t } => {
            assert_eq!(param, "missing_var");
            assert_eq!(t, template);
        }
        other => panic!("expected MissingParameter, got {other:?}"),
    }
}

#[test]
fn test_interpolation_empty_template_and_whitespace() {
    let params = BTreeMap::new();
    let res = PromptEngine::interpolate("", &params).expect("empty template succeeds");
    assert_eq!(res, "");

    let res_ws = PromptEngine::interpolate("   \n\t  ", &params).expect("whitespace succeeds");
    assert_eq!(res_ws, "   \n\t  ");
}

#[test]
fn test_interpolation_unclosed_and_unmatched_braces() {
    let params = BTreeMap::new();
    let template_unclosed = "Hello {{ unclosed variable world";
    let res = PromptEngine::interpolate(template_unclosed, &params).expect("unclosed brace ok");
    assert_eq!(res, "Hello {{ unclosed variable world");

    let template_unmatched = "Hello unmatched }} variable world";
    let res2 = PromptEngine::interpolate(template_unmatched, &params).expect("unmatched brace ok");
    assert_eq!(res2, "Hello unmatched }} variable world");
}

#[test]
fn test_interpolation_adjacent_and_repeated_variables() {
    let mut params = BTreeMap::new();
    params.insert("a".to_owned(), "ALPHA".to_owned());
    params.insert("b".to_owned(), "BETA".to_owned());

    let template_adjacent = "{{a}}{{b}}";
    let res = PromptEngine::interpolate(template_adjacent, &params).expect("adjacent braces");
    assert_eq!(res, "ALPHABETA");

    let template_repeated = "{{a}} then {{b}} then {{a}} again";
    let res2 = PromptEngine::interpolate(template_repeated, &params).expect("repeated braces");
    assert_eq!(res2, "ALPHA then BETA then ALPHA again");
}

#[test]
fn test_interpolation_anti_recursion_single_pass() {
    let mut params = BTreeMap::new();
    // Injected value contains a variable template token
    params.insert("task".to_owned(), "do {{malicious_expansion}}".to_owned());
    params.insert("malicious_expansion".to_owned(), "PWNED".to_owned());

    let template = "Task is: {{task}}.";
    let res = PromptEngine::interpolate(template, &params).expect("anti-recursion single-pass");
    // Value MUST NOT be recursively evaluated
    assert_eq!(res, "Task is: do {{malicious_expansion}}.");
    assert!(!res.contains("PWNED"));
}

#[test]
fn test_interpolation_large_strings() {
    let mut params = BTreeMap::new();
    let large_val = "X".repeat(100_000);
    params.insert("large".to_owned(), large_val.clone());
    params.insert("num".to_owned(), "42".to_owned());

    let mut template = String::new();
    for i in 0..500 {
        template.push_str(&format!("prefix_{i}_{{{{num}}}} "));
    }
    template.push_str("data: {{large}} end");

    let res = PromptEngine::interpolate(&template, &params).expect("large template interpolation");
    assert!(res.contains("prefix_0_42"));
    assert!(res.contains("prefix_499_42"));
    assert!(res.contains(&large_val));
}

#[test]
fn test_interpolation_nested_triple_braces_behavior() {
    // When a user writes {{{x}}}, find("{{") matches first two, after_open is {x}}}.
    // find("}}") finds the first two }}, after_open[..end_idx] is "{x".
    // This tests how the parser parses triple braces.
    let mut params = BTreeMap::new();
    params.insert("{x".to_owned(), "VAL".to_owned());

    let template = "{{{x}}}";
    let res = PromptEngine::interpolate(template, &params).expect("triple braces");
    // With {x in params, it replaces {{{x}} with VAL}
    assert_eq!(res, "VAL}");
}

// ---------------------------------------------------------------------------
// 2. Preset Priority Resolution & Case Shadowing
// ---------------------------------------------------------------------------

#[test]
fn test_preset_priority_workspace_over_user_over_builtin() {
    let temp_root = unique_temp_dir("priority_test");
    let ws_dir = temp_root.join("ws");
    let user_dir = temp_root.join("user");

    let engine = PromptEngine::open(&ws_dir, &user_dir);

    // Baseline: reviewer is Builtin
    let (b_preset, b_tier) = engine.get("reviewer").expect("get builtin");
    assert_eq!(b_tier, StorageTier::Builtin);
    assert_eq!(b_preset.name, "reviewer");

    // Add to User tier
    let user_preset = PromptPreset {
        name: "reviewer".to_owned(),
        description: Some("User tier reviewer".to_owned()),
        kind: PresetKind::Persona,
        content: "User tier content".to_owned(),
        parameters: Vec::new(),
        model: None,
        mode: None,
        created_at: 100,
        updated_at: 100,
    };
    engine.save(user_preset, false).expect("save to user tier");

    // Now get("reviewer") must be User tier
    let (u_preset, u_tier) = engine.get("reviewer").expect("get user");
    assert_eq!(u_tier, StorageTier::User);
    assert_eq!(u_preset.content, "User tier content");

    // Add to Workspace tier
    let ws_preset = PromptPreset {
        name: "reviewer".to_owned(),
        description: Some("Workspace tier reviewer".to_owned()),
        kind: PresetKind::Persona,
        content: "Workspace tier content".to_owned(),
        parameters: Vec::new(),
        model: None,
        mode: None,
        created_at: 200,
        updated_at: 200,
    };
    engine
        .save(ws_preset, true)
        .expect("save to workspace tier");

    // Now get("reviewer") must be Workspace tier
    let (w_preset, w_tier) = engine.get("reviewer").expect("get workspace");
    assert_eq!(w_tier, StorageTier::Workspace);
    assert_eq!(w_preset.content, "Workspace tier content");

    // Clean up temp dir
    let _ = fs::remove_dir_all(&temp_root);
}

#[test]
fn test_preset_case_shadowing_anomaly() {
    // CHALLENGE: If a user saves "Reviewer" (capital R) on User tier,
    // and "reviewer" (lowercase r) on Workspace tier,
    // because BTreeMap orders "Reviewer" < "reviewer" (ASCII R < r),
    // what does engine.get("reviewer") return?
    let temp_root = unique_temp_dir("case_shadowing_test");
    let ws_dir = temp_root.join("ws");
    let user_dir = temp_root.join("user");

    let engine = PromptEngine::open(&ws_dir, &user_dir);

    // Save "Reviewer" to User tier
    let user_preset = PromptPreset {
        name: "Reviewer".to_owned(),
        description: Some("User uppercase".to_owned()),
        kind: PresetKind::Persona,
        content: "USER CONTENT".to_owned(),
        parameters: Vec::new(),
        model: None,
        mode: None,
        created_at: 10,
        updated_at: 10,
    };
    engine.save(user_preset, false).expect("save user Reviewer");

    // Save "reviewer" to Workspace tier
    let ws_preset = PromptPreset {
        name: "reviewer".to_owned(),
        description: Some("Workspace lowercase".to_owned()),
        kind: PresetKind::Persona,
        content: "WORKSPACE CONTENT".to_owned(),
        parameters: Vec::new(),
        model: None,
        mode: None,
        created_at: 20,
        updated_at: 20,
    };
    engine.save(ws_preset, true).expect("save ws reviewer");

    // Contract: Workspace tier MUST take precedence over User tier regardless of name casing
    let (resolved, tier) = engine.get("reviewer").expect("get reviewer");
    assert_eq!(
        tier,
        StorageTier::Workspace,
        "BUG: Workspace tier failed to shadow User tier due to BTreeMap case ordering"
    );
    assert_eq!(resolved.content, "WORKSPACE CONTENT");

    let _ = fs::remove_dir_all(&temp_root);
}

// ---------------------------------------------------------------------------
// 3. Name Validation & Security Boundaries
// ---------------------------------------------------------------------------

#[test]
fn test_validate_preset_name_boundaries() {
    // Valid names
    assert!(validate_preset_name("reviewer").is_ok());
    assert!(validate_preset_name("audit-rust").is_ok());
    assert!(validate_preset_name("my_template_123").is_ok());

    // Invalid names
    assert!(validate_preset_name("").is_err());
    assert!(validate_preset_name("   ").is_err());
    assert!(validate_preset_name("../traversal").is_err());
    assert!(validate_preset_name("..\\windows").is_err());
    assert!(validate_preset_name("a/b").is_err());
    assert!(validate_preset_name("a b").is_err());
    assert!(validate_preset_name("a;rm").is_err());
    assert!(validate_preset_name("a$b").is_err());
}

#[test]
fn test_delete_missing_preset_name_validation() {
    // CHALLENGE: PromptEngine::delete does NOT call validate_preset_name!
    // Test if delete allows path traversal outside prompts directory.
    let temp_root = unique_temp_dir("delete_traversal_test");
    let ws_dir = temp_root.join("ws");
    let user_dir = temp_root.join("user");

    let engine = PromptEngine::open(&ws_dir, &user_dir);

    // Create a target file outside .pandora/prompts/ but within ws_dir
    let victim_file = ws_dir.join(".pandora").join("victim.json");
    fs::create_dir_all(ws_dir.join(".pandora")).expect("created .pandora");
    fs::write(&victim_file, b"{\"secret\": true}").expect("wrote victim");
    assert!(victim_file.exists());

    // Attempt traversal deletion: "../victim"
    // ws_prompts_dir is ws/.pandora/prompts.
    // ws/.pandora/prompts/../victim.json points to ws/.pandora/victim.json!
    let result = engine.delete("../victim", true);
    assert!(
        result.is_err(),
        "delete with traversal name must return Err"
    );
    assert!(
        victim_file.exists(),
        "SECURITY BUG: Victim file was deleted outside prompts directory via path traversal in delete()!"
    );

    let _ = fs::remove_dir_all(&temp_root);
}
