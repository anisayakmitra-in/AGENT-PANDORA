//! Steerable TUI Visual Graph Inspector (Memory & DAG Explorer)
//!
//! Grounded in mathematical foundations:
//! - Theorem 6: Force-directed Lyapunov stability under damped integration (gamma = 0.85)
//! - Energy minimization of spring-electrical Hamiltonian: E(x) = E_spring(x) + E_repulsion(x)
//! - Affine terminal viewport projection with 2:1 character cell aspect ratio compensation

#![forbid(unsafe_code)]

use std::collections::HashMap;

/// 2D Euclidean Coordinate Point and Vector arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point2D {
    pub x: f64,
    pub y: f64,
}

impl Point2D {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn dist(self, other: Self) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }

    pub fn norm(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    pub fn scale(self, factor: f64) -> Self {
        Self {
            x: self.x * factor,
            y: self.y * factor,
        }
    }
}

impl std::ops::Add for Point2D {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }
}

impl std::ops::Sub for Point2D {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }
}

/// Physical node state within the force-directed simulation.
#[derive(Debug, Clone)]
pub struct VisualNode {
    pub id: String,
    pub label: String,
    pub pos: Point2D,
    pub velocity: Point2D,
    pub mass: f64,
    pub degree: usize,
    pub pinned: bool,
}

/// Force-directed physical simulation engine enforcing Lyapunov stability (Theorem 6).
#[derive(Debug, Clone)]
pub struct ForceDirectedLayout {
    pub nodes: HashMap<String, VisualNode>,
    pub edges: Vec<(String, String)>,
    pub width: f64,
    pub height: f64,
    pub spring_k: f64,
    pub damping: f64, // gamma = 0.85
}

impl ForceDirectedLayout {
    pub fn new(width: f64, height: f64) -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
            width: width.max(10.0),
            height: height.max(10.0),
            spring_k: 15.0,
            damping: 0.85,
        }
    }

    pub fn add_node(&mut self, id: &str, label: &str, initial_x: f64, initial_y: f64) {
        let node = VisualNode {
            id: id.to_string(),
            label: label.to_string(),
            pos: Point2D::new(initial_x, initial_y),
            velocity: Point2D::new(0.0, 0.0),
            mass: 1.0,
            degree: 0,
            pinned: false,
        };
        self.nodes.insert(id.to_string(), node);
        self.recompute_natural_spring_k();
    }

    pub fn add_edge(&mut self, u: &str, v: &str) {
        if self.nodes.contains_key(u) && self.nodes.contains_key(v) && u != v {
            self.edges.push((u.to_string(), v.to_string()));
            if let Some(nu) = self.nodes.get_mut(u) {
                nu.degree += 1;
            }
            if let Some(nv) = self.nodes.get_mut(v) {
                nv.degree += 1;
            }
        }
    }

    /// Optimal spring constant k = C * sqrt(Area / |V|)
    fn recompute_natural_spring_k(&mut self) {
        let n = self.nodes.len().max(1) as f64;
        let area = self.width * self.height;
        self.spring_k = 0.75 * (area / n).sqrt();
    }

    /// Single numerical step of velocity-damped force integration:
    /// v(t + dt) = gamma * v(t) + F * dt
    /// x(t + dt) = x(t) + v(t + dt) * dt
    pub fn step(&mut self, dt: f64) -> f64 {
        let k = self.spring_k.max(1.0);
        let k2 = k * k;
        let mut forces: HashMap<String, Point2D> = HashMap::new();

        // 1. Repulsive forces: F_r(u, v) = -(k^2 / d) * (x_v - x_u)/d
        let node_keys: Vec<String> = self.nodes.keys().cloned().collect();
        for i in 0..node_keys.len() {
            let u_id = &node_keys[i];
            let pos_u = self.nodes[u_id].pos;
            for v_id in node_keys.iter().skip(i + 1) {
                let pos_v = self.nodes[v_id].pos;
                let delta = pos_v - pos_u;
                let dist = delta.norm().max(0.1); // avoid division by zero

                let rep_mag = k2 / (dist * dist);
                let f_vec = delta.scale(rep_mag / dist);

                *forces.entry(u_id.clone()).or_default() =
                    *forces.entry(u_id.clone()).or_default() - f_vec;
                *forces.entry(v_id.clone()).or_default() =
                    *forces.entry(v_id.clone()).or_default() + f_vec;
            }
        }

        // 2. Attractive spring forces: F_a(u, v) = (d^2 / k) * (x_v - x_u)/d
        for (u, v) in &self.edges {
            if let (Some(nu), Some(nv)) = (self.nodes.get(u), self.nodes.get(v)) {
                let delta = nv.pos - nu.pos;
                let dist = delta.norm().max(0.1);

                let att_mag = (dist * dist) / k;
                let f_vec = delta.scale(att_mag / dist);

                *forces.entry(u.clone()).or_default() =
                    *forces.entry(u.clone()).or_default() + f_vec;
                *forces.entry(v.clone()).or_default() =
                    *forces.entry(v.clone()).or_default() - f_vec;
            }
        }

        // 3. Center gravitational force to prevent drifting
        let center = Point2D::new(self.width / 2.0, self.height / 2.0);
        for (id, node) in &self.nodes {
            let to_center = center - node.pos;
            let grav = to_center.scale(0.02);
            *forces.entry(id.clone()).or_default() = *forces.entry(id.clone()).or_default() + grav;
        }

        // 4. Numerical integration with velocity damping (gamma = 0.85)
        let mut total_kinetic_energy = 0.0;

        for (id, node) in self.nodes.iter_mut() {
            if node.pinned {
                node.velocity = Point2D::new(0.0, 0.0);
                continue;
            }

            let force = forces.get(id).copied().unwrap_or_default();
            // v(t+dt) = gamma * v(t) + (F / m) * dt
            node.velocity = node.velocity.scale(self.damping) + force.scale(dt / node.mass);

            // Velocity capping for numerical stability
            let max_v = 15.0;
            let v_norm = node.velocity.norm();
            if v_norm > max_v {
                node.velocity = node.velocity.scale(max_v / v_norm);
            }

            // x(t+dt) = x(t) + v(t+dt) * dt
            node.pos = node.pos + node.velocity.scale(dt);

            // Bounded box containment
            node.pos.x = node.pos.x.clamp(2.0, self.width - 2.0);
            node.pos.y = node.pos.y.clamp(2.0, self.height - 2.0);

            total_kinetic_energy += 0.5 * node.mass * node.velocity.norm().powi(2);
        }

        total_kinetic_energy
    }

    /// Run simulation until mechanical equilibrium: total kinetic energy K < epsilon.
    pub fn simulate_until_equilibrium(&mut self, max_iterations: usize, epsilon: f64) -> usize {
        let dt = 0.1;
        for iter in 0..max_iterations {
            let ke = self.step(dt);
            if ke < epsilon {
                return iter + 1;
            }
        }
        max_iterations
    }

    /// Calculate total spring-electrical potential energy E(x).
    pub fn potential_energy(&self) -> f64 {
        let k = self.spring_k.max(1.0);
        let k2 = k * k;
        let mut energy = 0.0;

        // E_spring = sum_{(u,v)} 0.5 * (d^2 / k)
        for (u, v) in &self.edges {
            if let (Some(nu), Some(nv)) = (self.nodes.get(u), self.nodes.get(v)) {
                let d = nu.pos.dist(nv.pos);
                energy += 0.5 * (d * d) / k;
            }
        }

        // E_repulsion = sum_{u < v} (k^2 / d)
        let node_keys: Vec<String> = self.nodes.keys().cloned().collect();
        for i in 0..node_keys.len() {
            let u_id = &node_keys[i];
            let pos_u = self.nodes[u_id].pos;
            for v_id in node_keys.iter().skip(i + 1) {
                let pos_v = self.nodes[v_id].pos;
                let d = pos_u.dist(pos_v).max(0.1);
                energy += k2 / d;
            }
        }

        energy
    }
}

// ============================================================================
// Viewport Camera with 2:1 Terminal Aspect Ratio Compensation
// ============================================================================

/// Interactive viewport camera enabling steering (Pan, Zoom, Focus).
#[derive(Debug, Clone)]
pub struct ViewportCamera {
    pub pan_x: f64,
    pub pan_y: f64,
    pub zoom: f64,
    pub aspect_ratio: f64, // 2.0 for standard character cells (2w = 1h)
}

impl Default for ViewportCamera {
    fn default() -> Self {
        Self {
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
            aspect_ratio: 2.0,
        }
    }
}

impl ViewportCamera {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.pan_x += dx / self.zoom;
        self.pan_y += dy / self.zoom;
    }

    pub fn zoom_in(&mut self) {
        self.zoom = (self.zoom * 1.25).min(10.0);
    }

    pub fn zoom_out(&mut self) {
        self.zoom = (self.zoom / 1.25).max(0.2);
    }

    pub fn focus_on(&mut self, point: Point2D) {
        self.pan_x = point.x;
        self.pan_y = point.y;
    }

    /// Map continuous world coordinate to discrete terminal character cell (col, row).
    pub fn world_to_screen(
        &self,
        world: Point2D,
        screen_w: usize,
        screen_h: usize,
    ) -> Option<(usize, usize)> {
        let center_x = (screen_w as f64) / 2.0;
        let center_y = (screen_h as f64) / 2.0;

        // Apply 2:1 character cell aspect ratio calibration
        let sx = self.zoom * self.aspect_ratio;
        let sy = self.zoom;

        let col_f = center_x + sx * (world.x - self.pan_x);
        let row_f = center_y + sy * (world.y - self.pan_y);

        if col_f >= 0.0 && col_f < (screen_w as f64) && row_f >= 0.0 && row_f < (screen_h as f64) {
            Some((col_f as usize, row_f as usize))
        } else {
            None
        }
    }
}

// ============================================================================
// ASCII Grid Rasterizer: Direct Terminal Matrix Rendering
// ============================================================================

/// Rasterizes graph topologies directly into terminal text buffers.
pub struct AsciiGridRenderer;

impl AsciiGridRenderer {
    pub fn render(
        layout: &ForceDirectedLayout,
        camera: &ViewportCamera,
        width: usize,
        height: usize,
    ) -> Vec<String> {
        let mut grid: Vec<Vec<char>> = vec![vec![' '; width]; height];

        // 1. Rasterize edges with Bresenham line algorithm
        for (u_id, v_id) in &layout.edges {
            if let (Some(u), Some(v)) = (layout.nodes.get(u_id), layout.nodes.get(v_id)) {
                let p1 = camera.world_to_screen(u.pos, width, height);
                let p2 = camera.world_to_screen(v.pos, width, height);
                if let (Some((x0, y0)), Some((x1, y1))) = (p1, p2) {
                    Self::draw_line(
                        &mut grid,
                        x0 as isize,
                        y0 as isize,
                        x1 as isize,
                        y1 as isize,
                    );
                }
            }
        }

        // 2. Rasterize nodes and labels
        for node in layout.nodes.values() {
            if let Some((cx, cy)) = camera.world_to_screen(node.pos, width, height) {
                let label = format!("[{}]", node.label);
                let start_x = cx.saturating_sub(label.len() / 2);
                for (offset, ch) in label.chars().enumerate() {
                    let px = start_x + offset;
                    if px < width && cy < height {
                        grid[cy][px] = ch;
                    }
                }
            }
        }

        grid.into_iter()
            .map(|row| row.into_iter().collect())
            .collect()
    }

    fn draw_line(grid: &mut [Vec<char>], mut x0: isize, mut y0: isize, x1: isize, y1: isize) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        let height = grid.len() as isize;
        let width = if height > 0 {
            grid[0].len() as isize
        } else {
            0
        };

        loop {
            if x0 >= 0 && x0 < width && y0 >= 0 && y0 < height {
                let cell = &mut grid[y0 as usize][x0 as usize];
                if *cell == ' ' {
                    *cell = '.';
                }
            }
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }
}
