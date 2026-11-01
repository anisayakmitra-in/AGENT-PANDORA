#![forbid(unsafe_code)]

use pandora_harnesses::verification::{
    ScreenshotArtifact, VerificationGene, VisualStep, VisualVerificationGene,
    assert_element_visibility, assert_pixel_tolerance, assert_screenshot_hash, assert_text_matches,
    is_verification_gene, verification_static_output,
};
use pandora_runtime::mock_cdp::MockCdpServer;
use pandora_runtime::synthetic_canvas::{PNG_SIGNATURE, SyntheticCanvas, adler32, crc32};
use pandora_runtime::tool_engine::{ToolEngine, ToolProfile};
use pandora_types::Gene;
use serde_json::json;

#[test]
fn test_png_rfc_compliance_and_canvas_rasterizer() {
    let mut canvas = SyntheticCanvas::new_white(320, 240);
    assert_eq!(canvas.width(), 320);
    assert_eq!(canvas.height(), 240);

    // Render a button box
    canvas.draw_rect(
        20,
        30,
        120,
        40,
        Some((59, 130, 246, 255)),     // Blue fill
        Some(((30, 64, 175, 255), 2)), // Darker blue border
    );

    // Render text inside button
    canvas.draw_text(30, 42, "Submit", (255, 255, 255, 255), 1);

    // Verify interior and text pixels
    assert_eq!(canvas.get_pixel(20, 30), (30, 64, 175, 255)); // Border pixel
    let mut found_white_text = false;
    for py in 42..52 {
        for px in 30..80 {
            if canvas.get_pixel(px, py) == (255, 255, 255, 255) {
                found_white_text = true;
                break;
            }
        }
    }
    assert!(found_white_text, "text pixels must be rasterized in white");

    // Test PNG encoding
    let png_bytes = canvas.to_png_bytes();
    assert!(png_bytes.len() > 100);
    assert_eq!(&png_bytes[0..8], &PNG_SIGNATURE);

    // Verify IHDR chunk
    assert_eq!(&png_bytes[12..16], b"IHDR");
    let w = u32::from_be_bytes([png_bytes[16], png_bytes[17], png_bytes[18], png_bytes[19]]);
    let h = u32::from_be_bytes([png_bytes[20], png_bytes[21], png_bytes[22], png_bytes[23]]);
    assert_eq!(w, 320);
    assert_eq!(h, 240);
    assert_eq!(png_bytes[24], 8); // 8-bit depth
    assert_eq!(png_bytes[25], 6); // RGBA color type

    // Verify IEND chunk exists at end of file
    let len = png_bytes.len();
    assert_eq!(&png_bytes[len - 8..len - 4], b"IEND");

    // Base64 and SHA-256 checks
    let base64_png = canvas.to_base64_png();
    assert!(base64_png.len() > 100);
    let digest = canvas.sha256_digest();
    assert_eq!(digest.len(), 64);
    assert!(assert_screenshot_hash(&digest, &digest).is_ok());

    // Check Adler-32 and CRC-32 primitives
    assert_eq!(crc32(b"123456789"), 0xCBF43926);
    assert_eq!(adler32(b"123456789"), 0x091E01DE);
}

#[test]
fn test_mock_cdp_server_json_rpc_and_domains() {
    let mut server = MockCdpServer::new(1280, 720);

    // Test Page domain
    let page_enable = server.handle_json_rpc(r#"{"id": 1, "method": "Page.enable"}"#);
    assert!(page_enable.contains(r#""id":1"#));

    // Page.navigate with realistic HTML
    let html = r#"
        <!DOCTYPE html>
        <html>
            <head><title>AGENT-PANDORA Console</title></head>
            <body>
                <header>
                    <h1 id="app-title">AGENT-PANDORA v3.0.1</h1>
                </header>
                <main class="container">
                    <form id="auth-form">
                        <label for="username">Username</label>
                        <input id="username" type="text" placeholder="Enter username" />
                        <button id="submit-btn" class="btn btn-primary">Sign In</button>
                    </form>
                </main>
            </body>
        </html>
    "#;
    let nav_req = json!({
        "id": 2,
        "method": "Page.navigate",
        "params": { "html": html }
    })
    .to_string();
    let nav_res = server.handle_json_rpc(&nav_req);
    assert!(nav_res.contains("pandora-frame-1"));

    // DOM.getDocument
    let get_doc = server.handle_json_rpc(r#"{"id": 3, "method": "DOM.getDocument"}"#);
    assert!(get_doc.contains("AGENT-PANDORA"));
    assert!(get_doc.contains(r#""nodeName":"#));

    // DOM.querySelector
    let query_res = server.handle_json_rpc(
        r##"{"id": 4, "method": "DOM.querySelector", "params": {"selector": "#submit-btn"}}"##,
    );
    assert!(query_res.contains(r#""nodeId""#));

    // Input.dispatchKeyEvent (type username)
    server.document_mut().click("#username").unwrap();
    let type_req = json!({
        "id": 5,
        "method": "Input.dispatchKeyEvent",
        "params": { "type": "keyDown", "text": "pandora_operator" }
    })
    .to_string();
    let type_res = server.handle_json_rpc(&type_req);
    assert!(type_res.contains(r#""result":{}"#));

    // Runtime.evaluate to verify value was updated
    let eval_res = server.handle_json_rpc(
        r##"{"id": 6, "method": "Runtime.evaluate", "params": {"expression": "document.querySelector('#username').value"}}"##,
    );
    assert!(eval_res.contains("pandora_operator"));

    // Page.captureScreenshot
    let ss_res = server.handle_json_rpc(r#"{"id": 7, "method": "Page.captureScreenshot"}"#);
    assert!(ss_res.contains(r#""data":""#));

    // Emulation.setDeviceMetricsOverride
    let emu_res = server.handle_json_rpc(
        r#"{"id": 8, "method": "Emulation.setDeviceMetricsOverride", "params": {"width": 375, "height": 667, "mobile": true}}"#,
    );
    assert!(emu_res.contains(r#""result":{}"#));
    assert_eq!(server.document().viewport_width, 375);
    assert_eq!(server.document().viewport_height, 667);
    assert!(server.is_mobile());
}

#[test]
fn test_visual_verification_flow_and_assertions() {
    let mut server = MockCdpServer::new(1280, 720);

    let html = r#"
        <html>
            <head><title>Visual Verification Suite</title></head>
            <body>
                <h1 id="title">Settings</h1>
                <div class="panel">
                    <input id="dark-mode" type="checkbox" />
                    <button id="save-btn">Save Changes</button>
                    <p id="status-msg">All systems operational</p>
                </div>
            </body>
        </html>
    "#;

    let steps = vec![
        VisualStep {
            step_type: "navigate".to_string(),
            url: Some(html.to_string()),
            selector: None,
            text: None,
            name: None,
            expression: None,
            expected_hash: None,
            expected_text: None,
            expected_value: None,
            tolerance_percent: None,
        },
        VisualStep {
            step_type: "assert_visible".to_string(),
            url: None,
            selector: Some("#title".to_string()),
            text: None,
            name: None,
            expression: None,
            expected_hash: None,
            expected_text: None,
            expected_value: None,
            tolerance_percent: None,
        },
        VisualStep {
            step_type: "assert_text".to_string(),
            url: None,
            selector: Some("#status-msg".to_string()),
            text: Some("All systems operational".to_string()),
            name: None,
            expression: None,
            expected_hash: None,
            expected_text: Some("All systems operational".to_string()),
            expected_value: None,
            tolerance_percent: None,
        },
        VisualStep {
            step_type: "click".to_string(),
            url: None,
            selector: Some("#dark-mode".to_string()),
            text: None,
            name: None,
            expression: None,
            expected_hash: None,
            expected_text: None,
            expected_value: None,
            tolerance_percent: None,
        },
        VisualStep {
            step_type: "assert_eval".to_string(),
            url: None,
            selector: None,
            text: None,
            name: None,
            expression: Some("document.querySelector('#dark-mode').checked".to_string()),
            expected_hash: None,
            expected_text: None,
            expected_value: Some(json!(true)),
            tolerance_percent: None,
        },
        VisualStep {
            step_type: "screenshot".to_string(),
            url: None,
            selector: None,
            text: None,
            name: Some("settings_saved".to_string()),
            expression: None,
            expected_hash: None,
            expected_text: None,
            expected_value: None,
            tolerance_percent: None,
        },
    ];

    let report = server.execute_visual_steps(&steps).expect("steps executed");
    assert!(report.passed, "Visual verification steps must pass");
    assert_eq!(report.steps_total, 6);
    assert_eq!(report.steps_passed, 6);
    assert_eq!(report.assertions_passed, 3);
    assert_eq!(report.assertions_failed, 0);
    assert_eq!(report.screenshots.len(), 1);
    assert_eq!(report.screenshots[0].name, "settings_saved");
    assert_eq!(report.screenshots[0].width, 1280);
    assert_eq!(report.screenshots[0].height, 720);
    assert!(!report.screenshots[0].base64_png.is_empty());

    // Additional assertion tests
    assert!(assert_element_visibility(true, true).is_ok());
    assert!(assert_text_matches("All systems operational", "operational", false).is_ok());

    let buf1 = vec![0u8; 16];
    let buf2 = vec![0u8; 16];
    assert!(assert_pixel_tolerance(&buf1, &buf2, 0.0).is_ok());

    let artifact = ScreenshotArtifact {
        name: "test".into(),
        width: 100,
        height: 100,
        sha256: "hash".into(),
        base64_png: "b64".into(),
    };
    assert_eq!(artifact.name, "test");
}

#[test]
fn test_tool_engine_registration_and_profiles() {
    let engine = ToolEngine::with_builtins();

    // Verify verification.visual is registered
    let visual_tool = engine
        .list()
        .into_iter()
        .find(|t| t.id().as_str() == "verification.visual")
        .expect("verification.visual must be registered in ToolEngine");
    assert_eq!(visual_tool.version(), "1.0.0");

    // Verify it is accessible in Full and Coding profiles
    let full_tools = engine.list_filtered(ToolProfile::Full, None);
    assert!(
        full_tools
            .iter()
            .any(|t| t.id().as_str() == "verification.visual")
    );

    let coding_tools = engine.list_filtered(ToolProfile::Coding, None);
    assert!(
        coding_tools
            .iter()
            .any(|t| t.id().as_str() == "verification.visual")
    );

    // Prepare invocation
    let invocation = engine
        .prepare_invocation("verification.visual", &json!({"action": "visual_verify"}))
        .expect("invocation preparation succeeds");
    assert_eq!(
        invocation.task().summary(),
        "verification.visual:visual_verify"
    );
}

#[test]
fn test_verification_gene_role_and_harness_integration() {
    // Check is_verification_gene recognizes verification.visual
    let visual_gene_id = pandora_types::GeneId::new("verification.visual").unwrap();
    assert!(is_verification_gene(&visual_gene_id));

    // Check static output / guide
    let guide = verification_static_output(&visual_gene_id).expect("guide available");
    assert!(guide.contains("CDP"));

    // Check VisualVerificationGene construction
    let gene = VisualVerificationGene::new().expect("visual gene created");
    assert_eq!(gene.manifest().id().as_str(), "verification.visual");

    // Check VerificationGene::all includes Visual
    let all = VerificationGene::all();
    assert_eq!(all.len(), 5);
    assert!(
        all.iter()
            .any(|g| g.manifest().id().as_str() == "verification.visual")
    );
}
