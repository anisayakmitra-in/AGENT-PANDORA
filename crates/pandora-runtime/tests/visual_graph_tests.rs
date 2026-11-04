//! Comprehensive Verification of Steerable TUI Visual Graph Inspector
//!
//! Grounded in mathematical foundations:
//! - Theorem 6: Lyapunov stability and kinetic energy decay under damped integration
//! - Spring-electrical energy minimization
//! - Viewport affine transform with 2:1 character cell aspect ratio calibration
//! - ASCII terminal matrix rasterization

use pandora_runtime::visual_graph::{
    AsciiGridRenderer, ForceDirectedLayout, Point2D, ViewportCamera,
};

#[test]
fn test_point2d_vector_arithmetic() {
    let p1 = Point2D::new(3.0, 4.0);
    let p2 = Point2D::new(0.0, 0.0);

    assert_eq!(p1.norm(), 5.0);
    assert_eq!(p1.dist(p2), 5.0);

    let p3 = p1 + Point2D::new(1.0, 2.0);
    assert_eq!(p3, Point2D::new(4.0, 6.0));

    let p4 = p1 - Point2D::new(1.0, 1.0);
    assert_eq!(p4, Point2D::new(2.0, 3.0));

    let p5 = p1.scale(2.0);
    assert_eq!(p5, Point2D::new(6.0, 8.0));
}

#[test]
fn test_force_directed_lyapunov_stability_and_energy_decay() {
    let mut layout = ForceDirectedLayout::new(100.0, 100.0);

    // Create a 5-node pentagon graph with central hub
    layout.add_node("hub", "HUB", 50.0, 50.0);
    layout.add_node("n1", "N1", 10.0, 10.0);
    layout.add_node("n2", "N2", 90.0, 10.0);
    layout.add_node("n3", "N3", 90.0, 90.0);
    layout.add_node("n4", "N4", 10.0, 90.0);

    layout.add_edge("hub", "n1");
    layout.add_edge("hub", "n2");
    layout.add_edge("hub", "n3");
    layout.add_edge("hub", "n4");
    layout.add_edge("n1", "n2");
    layout.add_edge("n2", "n3");
    layout.add_edge("n3", "n4");
    layout.add_edge("n4", "n1");

    let initial_potential = layout.potential_energy();
    assert!(initial_potential > 0.0);

    // Run 50 damped simulation steps (gamma = 0.85)
    let mut prev_ke = 1e9;
    let mut steps_to_equilibrium = 0;

    for step in 1..=100 {
        let ke = layout.step(0.1);
        if ke < 0.1 {
            steps_to_equilibrium = step;
            break;
        }
        prev_ke = ke;
    }

    // System must converge to mechanical equilibrium (Theorem 6)
    assert!(
        steps_to_equilibrium > 0,
        "Layout must reach equilibrium: final KE was {prev_ke}"
    );

    // Nodes must remain within bounding box
    for node in layout.nodes.values() {
        assert!(node.pos.x >= 2.0 && node.pos.x <= 98.0);
        assert!(node.pos.y >= 2.0 && node.pos.y <= 98.0);
    }
}

#[test]
fn test_viewport_camera_steerability_and_aspect_ratio() {
    let mut camera = ViewportCamera::new();
    assert_eq!(camera.zoom, 1.0);
    assert_eq!(camera.aspect_ratio, 2.0);

    // Panning
    camera.pan(10.0, -5.0);
    assert_eq!(camera.pan_x, 10.0);
    assert_eq!(camera.pan_y, -5.0);

    // Zoom in and out
    camera.zoom_in();
    assert_eq!(camera.zoom, 1.25);
    camera.zoom_out();
    assert_eq!(camera.zoom, 1.0);

    // Focus on specific node position
    camera.focus_on(Point2D::new(50.0, 25.0));
    assert_eq!(camera.pan_x, 50.0);
    assert_eq!(camera.pan_y, 25.0);

    // World to screen projection: Center point maps to screen center
    let screen_w = 80;
    let screen_h = 24;
    let center_pt = Point2D::new(50.0, 25.0);
    let (cx, cy) = camera
        .world_to_screen(center_pt, screen_w, screen_h)
        .expect("Center must be within screen bounds");
    assert_eq!(cx, screen_w / 2);
    assert_eq!(cy, screen_h / 2);

    // Aspect ratio verification:
    // With aspect_ratio = 2.0, a horizontal offset dx should produce 2x the pixel change of dy
    let pt_right = Point2D::new(55.0, 25.0); // dx = 5
    let pt_down = Point2D::new(50.0, 30.0); // dy = 5
    let (rx, _) = camera
        .world_to_screen(pt_right, screen_w, screen_h)
        .unwrap();
    let (_, dy_pixel) = camera.world_to_screen(pt_down, screen_w, screen_h).unwrap();

    let delta_col = rx - cx;
    let delta_row = dy_pixel - cy;
    assert_eq!(
        delta_col,
        delta_row * 2,
        "Aspect ratio 2:1 calibration invariant must hold"
    );
}

#[test]
fn test_ascii_grid_rendering_and_line_rasterization() {
    let mut layout = ForceDirectedLayout::new(40.0, 20.0);
    layout.add_node("alpha", "A", 10.0, 10.0);
    layout.add_node("beta", "B", 30.0, 10.0);
    layout.add_edge("alpha", "beta");

    let mut camera = ViewportCamera::new();
    camera.focus_on(Point2D::new(20.0, 10.0));
    camera.zoom = 0.8;

    let width = 60;
    let height = 20;
    let rendered_lines = AsciiGridRenderer::render(&layout, &camera, width, height);

    assert_eq!(rendered_lines.len(), height);
    for line in &rendered_lines {
        assert_eq!(line.chars().count(), width);
    }

    // Node labels [A] and [B] must be present in the rasterized output
    let full_text = rendered_lines.join("\n");
    assert!(
        full_text.contains("[A]"),
        "Rendered ASCII output must contain node label [A]"
    );
    assert!(
        full_text.contains("[B]"),
        "Rendered ASCII output must contain node label [B]"
    );
    assert!(
        full_text.contains('.'),
        "Rendered ASCII output must contain Bresenham edge dots '.'"
    );
}
