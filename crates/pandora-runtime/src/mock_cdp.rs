//! Self-Contained Mock Chrome DevTools Protocol (CDP) JSON-RPC Server and Virtual DOM Engine.
//!
//! Provides a headless browser automation harness for autonomous web testing without requiring
//! external browser binaries (Chromium, Chrome, or Playwright).
//! Handles CDP domains: Page, DOM, Runtime, Input, Emulation.
//! Strict `#![forbid(unsafe_code)]` compliance.

use crate::synthetic_canvas::SyntheticCanvas;
use pandora_harnesses::verification::{
    ScreenshotArtifact, VisualStep, VisualVerificationReport,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

/// Result type for CDP operations.
pub type CdpResult<T> = Result<T, CdpError>;

/// CDP errors.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CdpError {
    InvalidRequest(String),
    MethodNotFound(String),
    NodeNotFound(u32),
    SelectorNotFound(String),
    EvaluationFailed(String),
    InvalidParameters(String),
    Internal(String),
}

impl std::fmt::Display for CdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRequest(msg) => write!(f, "CDP invalid request: {msg}"),
            Self::MethodNotFound(method) => write!(f, "CDP method not found: {method}"),
            Self::NodeNotFound(id) => write!(f, "CDP node not found: {id}"),
            Self::SelectorNotFound(sel) => write!(f, "CDP selector not found: {sel}"),
            Self::EvaluationFailed(msg) => write!(f, "CDP evaluation failed: {msg}"),
            Self::InvalidParameters(msg) => write!(f, "CDP invalid parameters: {msg}"),
            Self::Internal(msg) => write!(f, "CDP internal error: {msg}"),
        }
    }
}

impl std::error::Error for CdpError {}

/// Bounding rectangle in virtual pixels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ElementRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl ElementRect {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(&self, px: u32, py: u32) -> bool {
        px >= self.x
            && px < self.x.saturating_add(self.width)
            && py >= self.y
            && py < self.y.saturating_add(self.height)
    }
}

/// A node in the headless virtual DOM tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VirtualNode {
    pub id: u32,
    pub parent: Option<u32>,
    pub children: Vec<u32>,
    pub tag: String,
    pub attributes: HashMap<String, String>,
    pub text_content: String,
    pub bounds: ElementRect,
    pub is_focused: bool,
    pub is_checked: bool,
    pub is_visible: bool,
    pub click_count: u32,
}

impl VirtualNode {
    pub fn new(id: u32, tag: impl Into<String>) -> Self {
        Self {
            id,
            parent: None,
            children: Vec::new(),
            tag: tag.into().to_lowercase(),
            attributes: HashMap::new(),
            text_content: String::new(),
            bounds: ElementRect::default(),
            is_focused: false,
            is_checked: false,
            is_visible: true,
            click_count: 0,
        }
    }

    pub fn get_attribute(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(|s| s.as_str())
    }

    pub fn set_attribute(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.attributes.insert(name.into(), value.into());
    }

    pub fn matches_tag(&self, tag: &str) -> bool {
        tag == "*" || self.tag.eq_ignore_ascii_case(tag)
    }

    pub fn matches_id(&self, id: &str) -> bool {
        self.get_attribute("id").is_some_and(|v| v == id)
    }

    pub fn matches_class(&self, class_name: &str) -> bool {
        self.get_attribute("class")
            .is_some_and(|classes| classes.split_whitespace().any(|c| c == class_name))
    }

    pub fn matches_attribute(&self, attr: &str, expected_val: Option<&str>) -> bool {
        match expected_val {
            Some(expected) => self.get_attribute(attr).is_some_and(|v| v == expected),
            None => self.attributes.contains_key(attr),
        }
    }
}

/// Headless Virtual Document with HTML parser and layout engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VirtualDocument {
    pub url: String,
    pub title: String,
    pub root_id: u32,
    pub nodes: HashMap<u32, VirtualNode>,
    next_node_id: u32,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

impl Default for VirtualDocument {
    fn default() -> Self {
        Self::new(1280, 720)
    }
}

impl VirtualDocument {
    pub fn new(viewport_width: u32, viewport_height: u32) -> Self {
        let mut nodes = HashMap::new();
        let root = VirtualNode::new(1, "#document");
        nodes.insert(1, root);

        let mut doc = Self {
            url: "about:blank".to_string(),
            title: "Blank Document".to_string(),
            root_id: 1,
            nodes,
            next_node_id: 2,
            viewport_width,
            viewport_height,
        };
        doc.load_html("<html><head><title>Blank Document</title></head><body></body></html>");
        doc
    }

    pub fn allocate_id(&mut self) -> u32 {
        let id = self.next_node_id;
        self.next_node_id += 1;
        id
    }

    /// Loads and parses an HTML string into the virtual DOM tree, computing bounding layout boxes.
    pub fn load_html(&mut self, html: &str) {
        self.nodes.clear();
        self.next_node_id = 1;

        let root_id = self.allocate_id();
        let mut root = VirtualNode::new(root_id, "#document");
        root.bounds = ElementRect::new(0, 0, self.viewport_width, self.viewport_height);
        self.nodes.insert(root_id, root);
        self.root_id = root_id;

        self.parse_and_attach(html, root_id);
        self.compute_layout();
    }

    fn parse_and_attach(&mut self, html: &str, parent_id: u32) {
        let mut cursor = 0;
        let bytes = html.as_bytes();
        let len = bytes.len();
        let mut current_parent = parent_id;
        let mut parent_stack = vec![parent_id];

        while cursor < len {
            if bytes[cursor] == b'<' {
                // Check if comment or doctype
                if html[cursor..].starts_with("<!--") {
                    if let Some(end) = html[cursor..].find("-->") {
                        cursor += end + 3;
                        continue;
                    }
                }
                if html[cursor..].to_ascii_lowercase().starts_with("<!doctype") {
                    if let Some(end) = html[cursor..].find('>') {
                        cursor += end + 1;
                        continue;
                    }
                }

                // Check closing tag </tag>
                if cursor + 1 < len && bytes[cursor + 1] == b'/' {
                    if let Some(end) = html[cursor..].find('>') {
                        if parent_stack.len() > 1 {
                            parent_stack.pop();
                            current_parent = *parent_stack.last().unwrap_or(&self.root_id);
                        }
                        cursor += end + 1;
                        continue;
                    }
                }

                // Opening tag <tag ...>
                if let Some(end) = html[cursor..].find('>') {
                    let tag_content = &html[cursor + 1..cursor + end].trim();
                    let is_self_closing = tag_content.ends_with('/');
                    let clean_tag = tag_content.trim_end_matches('/').trim();

                    let mut parts = clean_tag.split_whitespace();
                    let tag_name = parts.next().unwrap_or("").to_lowercase();

                    if !tag_name.is_empty() {
                        let node_id = self.allocate_id();
                        let mut node = VirtualNode::new(node_id, &tag_name);
                        node.parent = Some(current_parent);

                        // Parse attributes
                        let remainder = clean_tag[tag_name.len()..].trim();
                        parse_attributes(remainder, &mut node.attributes);

                        // Special void tags in HTML: input, img, br, hr, meta, link
                        let is_void = is_self_closing
                            || matches!(
                                tag_name.as_str(),
                                "input" | "img" | "br" | "hr" | "meta" | "link"
                            );

                        self.nodes.insert(node_id, node);
                        if let Some(parent) = self.nodes.get_mut(&current_parent) {
                            parent.children.push(node_id);
                        }

                        if !is_void {
                            parent_stack.push(node_id);
                            current_parent = node_id;
                        }
                    }
                    cursor += end + 1;
                    continue;
                }
            }

            // Text content outside tags
            let text_start = cursor;
            while cursor < len && bytes[cursor] != b'<' {
                cursor += 1;
            }
            let text = html[text_start..cursor].trim();
            if !text.is_empty() {
                if let Some(parent_node) = self.nodes.get_mut(&current_parent) {
                    if !parent_node.text_content.is_empty() {
                        parent_node.text_content.push(' ');
                    }
                    parent_node.text_content.push_str(text);

                    // Extract title if inside <title>
                    if parent_node.tag == "title" {
                        self.title = parent_node.text_content.clone();
                    }
                }
            }
        }
    }

    /// Computes realistic element layout bounds based on DOM hierarchy and viewport dimensions.
    pub fn compute_layout(&mut self) {
        let width = self.viewport_width;
        let height = self.viewport_height;

        let mut y_cursor = 20u32;
        let body_id = self.query_selector("body").unwrap_or(self.root_id);

        if let Some(body) = self.nodes.get_mut(&body_id) {
            body.bounds = ElementRect::new(0, 0, width, height);
        }

        // Layout children of body linearly in vertical document flow
        let children: Vec<u32> = self
            .nodes
            .get(&body_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();

        for child_id in children {
            y_cursor = self.layout_subtree(child_id, 20, y_cursor, width.saturating_sub(40));
        }
    }

    fn layout_subtree(&mut self, node_id: u32, x: u32, y: u32, max_w: u32) -> u32 {
        let mut cur_y = y;
        let children: Vec<u32>;
        let tag: String;
        let text_len: usize;

        if let Some(node) = self.nodes.get(&node_id) {
            children = node.children.clone();
            tag = node.tag.clone();
            text_len = node.text_content.len();
        } else {
            return cur_y;
        }

        let is_block = matches!(
            tag.as_str(),
            "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" | "form" | "section" | "nav" | "footer" | "header"
        );

        let elem_h = match tag.as_str() {
            "h1" => 48,
            "h2" => 40,
            "h3" => 32,
            "p" => 28,
            "button" => 36,
            "input" => 36,
            _ => {
                if children.is_empty() && text_len > 0 {
                    24
                } else {
                    0
                }
            }
        };

        let elem_w = match tag.as_str() {
            "button" => (80 + text_len as u32 * 8).min(max_w),
            "input" => 240u32.min(max_w),
            _ => {
                if is_block {
                    max_w
                } else {
                    (text_len as u32 * 8 + 16).min(max_w)
                }
            }
        };

        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.bounds = ElementRect::new(x, cur_y, elem_w, elem_h);
        }

        cur_y = cur_y.saturating_add(elem_h);

        // Layout nested children
        let mut child_y = cur_y;
        for child_id in children {
            child_y = self.layout_subtree(child_id, x + 16, child_y, max_w.saturating_sub(16));
        }

        if child_y > cur_y {
            // Expand container height to enclose children
            if let Some(node) = self.nodes.get_mut(&node_id) {
                node.bounds.height = child_y.saturating_sub(cur_y).saturating_add(elem_h);
            }
            cur_y = child_y;
        }

        // Add bottom margin
        cur_y.saturating_add(12)
    }

    /// Evaluates a CSS selector and returns the first matching node ID.
    pub fn query_selector(&self, selector: &str) -> Option<u32> {
        let results = self.query_selector_all(selector);
        results.into_iter().next()
    }

    /// Evaluates a CSS selector and returns all matching node IDs.
    pub fn query_selector_all(&self, selector: &str) -> Vec<u32> {
        let selector = selector.trim();
        if selector.is_empty() {
            return Vec::new();
        }

        // Handle comma-separated selectors: e.g. "button, input"
        if selector.contains(',') {
            let mut matches = Vec::new();
            for part in selector.split(',') {
                for id in self.query_selector_all(part.trim()) {
                    if !matches.contains(&id) {
                        matches.push(id);
                    }
                }
            }
            return matches;
        }

        // Handle descendant selectors: e.g. "form button"
        let parts: Vec<&str> = selector.split_whitespace().collect();
        if parts.len() > 1 {
            let mut current_pool: Vec<u32> = self.nodes.keys().copied().collect();
            for part in parts {
                current_pool = current_pool
                    .into_iter()
                    .filter(|&id| {
                        if let Some(node) = self.nodes.get(&id) {
                            match_single_selector(node, part)
                        } else {
                            false
                        }
                    })
                    .flat_map(|id| self.descendant_ids(id))
                    .collect();
            }
            return current_pool;
        }

        // Single component selector
        let mut results = Vec::new();
        for (&id, node) in &self.nodes {
            if match_single_selector(node, selector) {
                results.push(id);
            }
        }
        results.sort_unstable();
        results
    }

    fn descendant_ids(&self, parent_id: u32) -> Vec<u32> {
        let mut results = Vec::new();
        if let Some(parent) = self.nodes.get(&parent_id) {
            for &child_id in &parent.children {
                results.push(child_id);
                results.extend(self.descendant_ids(child_id));
            }
        }
        results
    }

    /// Simulates a user click on an element by selector, updating focus and state.
    pub fn click(&mut self, selector: &str) -> CdpResult<u32> {
        let node_id = self
            .query_selector(selector)
            .ok_or_else(|| CdpError::SelectorNotFound(selector.to_owned()))?;

        // Reset previous focus
        for node in self.nodes.values_mut() {
            node.is_focused = false;
        }

        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.is_focused = true;
            node.click_count += 1;
            if node.tag == "input" && node.get_attribute("type") == Some("checkbox") {
                node.is_checked = !node.is_checked;
                node.set_attribute("checked", if node.is_checked { "true" } else { "false" });
            }
            Ok(node_id)
        } else {
            Err(CdpError::NodeNotFound(node_id))
        }
    }

    /// Simulates keyboard input into an element by selector, appending text to its value.
    pub fn type_text(&mut self, selector: &str, text: &str) -> CdpResult<u32> {
        let node_id = self
            .query_selector(selector)
            .ok_or_else(|| CdpError::SelectorNotFound(selector.to_owned()))?;

        // Set focus
        for node in self.nodes.values_mut() {
            node.is_focused = false;
        }

        if let Some(node) = self.nodes.get_mut(&node_id) {
            node.is_focused = true;
            let mut current = node.attributes.get("value").cloned().unwrap_or_default();
            current.push_str(text);
            node.attributes.insert("value".to_string(), current);
            node.text_content.push_str(text);
            Ok(node_id)
        } else {
            Err(CdpError::NodeNotFound(node_id))
        }
    }

    /// Evaluates JavaScript expressions against the virtual document state.
    pub fn evaluate_js(&self, expression: &str) -> CdpResult<Value> {
        let expr = expression.trim();

        // 1. document.title
        if expr == "document.title" {
            return Ok(json!(self.title));
        }

        // 2. window.innerWidth / window.innerHeight
        if expr == "window.innerWidth" {
            return Ok(json!(self.viewport_width));
        }
        if expr == "window.innerHeight" {
            return Ok(json!(self.viewport_height));
        }

        // 3. document.querySelector('...').textContent / .value / .checked
        if expr.starts_with("document.querySelector(") {
            let query_end = expr
                .find(')')
                .ok_or_else(|| CdpError::EvaluationFailed("unterminated querySelector".into()))?;
            let raw_sel = &expr[23..query_end].trim();
            let sel = raw_sel.trim_matches(|c| c == '\'' || c == '"');

            let remainder = expr[query_end + 1..].trim();
            let node = self.query_selector(sel).and_then(|id| self.nodes.get(&id));

            if remainder == ".textContent" || remainder == ".innerText" {
                return Ok(json!(node.map(|n| n.text_content.as_str()).unwrap_or("")));
            }
            if remainder == ".value" {
                return Ok(json!(
                    node.and_then(|n| n.get_attribute("value")).unwrap_or("")
                ));
            }
            if remainder == ".checked" {
                return Ok(json!(node.is_some_and(|n| n.is_checked)));
            }
            if remainder == " !== null" || remainder == " != null" {
                return Ok(json!(node.is_some()));
            }
            if remainder == " === null" || remainder == " == null" {
                return Ok(json!(node.is_none()));
            }
            if remainder.is_empty() {
                return Ok(json!(node.is_some()));
            }
        }

        // 4. Boolean literal assertions
        if expr == "true" {
            return Ok(json!(true));
        }
        if expr == "false" {
            return Ok(json!(false));
        }

        // 5. Arithmetic / numeric
        if let Ok(num) = expr.parse::<f64>() {
            return Ok(json!(num));
        }

        Ok(json!(expr))
    }

    /// Renders the virtual DOM into the provided `SyntheticCanvas`.
    pub fn render_to_canvas(&self, canvas: &mut SyntheticCanvas) {
        // Base page background
        canvas.fill(250, 250, 252, 255);

        // Render nodes
        for node in self.nodes.values() {
            if !node.is_visible || node.bounds.width == 0 || node.bounds.height == 0 {
                continue;
            }

            match node.tag.as_str() {
                "h1" | "h2" | "h3" | "h4" => {
                    let scale = if node.tag == "h1" { 2 } else { 1 };
                    canvas.draw_text(
                        node.bounds.x,
                        node.bounds.y + 4,
                        &node.text_content,
                        (30, 41, 59, 255), // Slate-800
                        scale,
                    );
                }
                "p" | "span" | "label" => {
                    canvas.draw_text(
                        node.bounds.x,
                        node.bounds.y + 4,
                        &node.text_content,
                        (51, 65, 85, 255), // Slate-700
                        1,
                    );
                }
                "button" => {
                    let fill = if node.is_focused {
                        (37, 99, 235, 255) // Active Blue
                    } else {
                        (59, 130, 246, 255) // Normal Blue
                    };
                    canvas.draw_rect(
                        node.bounds.x,
                        node.bounds.y,
                        node.bounds.width,
                        node.bounds.height,
                        Some(fill),
                        Some(((29, 78, 216, 255), 1)),
                    );
                    canvas.draw_text(
                        node.bounds.x + 8,
                        node.bounds.y + 10,
                        &node.text_content,
                        (255, 255, 255, 255),
                        1,
                    );
                }
                "input" => {
                    let border_color = if node.is_focused {
                        (59, 130, 246, 255) // Focused border: Blue
                    } else {
                        (203, 213, 225, 255) // Unfocused border: Slate-300
                    };
                    canvas.draw_rect(
                        node.bounds.x,
                        node.bounds.y,
                        node.bounds.width,
                        node.bounds.height,
                        Some((255, 255, 255, 255)),
                        Some((border_color, 1)),
                    );

                    let val = node.get_attribute("value").unwrap_or("");
                    if !val.is_empty() {
                        canvas.draw_text(
                            node.bounds.x + 8,
                            node.bounds.y + 10,
                            val,
                            (15, 23, 42, 255),
                            1,
                        );
                    } else if let Some(placeholder) = node.get_attribute("placeholder") {
                        canvas.draw_text(
                            node.bounds.x + 8,
                            node.bounds.y + 10,
                            placeholder,
                            (148, 163, 184, 255),
                            1,
                        );
                    }
                }
                "div" | "form" | "section" | "nav" | "footer" | "header" => {
                    if node.get_attribute("class").is_some() || node.get_attribute("id").is_some()
                    {
                        canvas.draw_rect(
                            node.bounds.x,
                            node.bounds.y,
                            node.bounds.width,
                            node.bounds.height,
                            None,
                            Some(((226, 232, 240, 255), 1)),
                        );
                    }
                }
                _ => {}
            }
        }

        // Draw viewport status watermark
        let tag = format!("PANDORA-CDP: {}x{}", self.viewport_width, self.viewport_height);
        canvas.draw_text(
            self.viewport_width.saturating_sub(200),
            self.viewport_height.saturating_sub(16),
            &tag,
            (148, 163, 184, 255),
            1,
        );
    }
}

/// Parses HTML attribute pairs from a string into a HashMap.
fn parse_attributes(raw: &str, attrs: &mut HashMap<String, String>) {
    let mut chars = raw.char_indices().peekable();
    while let Some(&(i, ch)) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }

        // Parse key
        let key_start = i;
        while let Some(&(_, c)) = chars.peek() {
            if c == '=' || c.is_whitespace() || c == '/' || c == '>' {
                break;
            }
            chars.next();
        }
        let key_end = chars.peek().map(|&(ci, _)| ci).unwrap_or(raw.len());
        let key = raw[key_start..key_end].trim().to_lowercase();
        if key.is_empty() {
            break;
        }

        // Check if '=' follows
        while let Some(&(_, c)) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
            } else {
                break;
            }
        }

        if let Some(&(_, '=')) = chars.peek() {
            chars.next(); // Consume '='
            while let Some(&(_, c)) = chars.peek() {
                if c.is_whitespace() {
                    chars.next();
                } else {
                    break;
                }
            }

            // Value can be quoted or unquoted
            if let Some(&(_, quote @ ('"' | '\''))) = chars.peek() {
                chars.next(); // Consume quote
                let val_start = chars.peek().map(|&(ci, _)| ci).unwrap_or(raw.len());
                let mut val_end = val_start;
                while let Some(&(vi, vc)) = chars.peek() {
                    if vc == quote {
                        val_end = vi;
                        chars.next(); // Consume closing quote
                        break;
                    }
                    val_end = vi + vc.len_utf8();
                    chars.next();
                }
                let val = &raw[val_start..val_end];
                attrs.insert(key, val.to_string());
            } else {
                let val_start = chars.peek().map(|&(ci, _)| ci).unwrap_or(raw.len());
                let mut val_end = val_start;
                while let Some(&(vi, vc)) = chars.peek() {
                    if vc.is_whitespace() || vc == '>' || vc == '/' {
                        val_end = vi;
                        break;
                    }
                    val_end = vi + vc.len_utf8();
                    chars.next();
                }
                let val = &raw[val_start..val_end];
                attrs.insert(key, val.to_string());
            }
        } else {
            // Boolean attribute without value: e.g. "checked", "disabled"
            attrs.insert(key, "true".to_string());
        }
    }
}

/// Matches a single selector component (e.g. `tag`, `#id`, `.class`, `[attr=val]`, `tag#id.class`).
fn match_single_selector(node: &VirtualNode, selector: &str) -> bool {
    let selector = selector.trim();
    if selector.is_empty() {
        return false;
    }

    // 1. Tag with id/class: e.g. "button#submit" or "div.main"
    if selector.contains('#') {
        let parts: Vec<&str> = selector.split('#').collect();
        let tag = parts[0];
        let id = parts[1];
        if !tag.is_empty() && !node.matches_tag(tag) {
            return false;
        }
        return node.matches_id(id);
    }

    if selector.contains('.') && !selector.starts_with('.') {
        let parts: Vec<&str> = selector.split('.').collect();
        let tag = parts[0];
        if !tag.is_empty() && !node.matches_tag(tag) {
            return false;
        }
        for class in &parts[1..] {
            if !node.matches_class(class) {
                return false;
            }
        }
        return true;
    }

    // 2. ID selector: "#some-id"
    if let Some(id) = selector.strip_prefix('#') {
        return node.matches_id(id);
    }

    // 3. Class selector: ".some-class"
    if let Some(class) = selector.strip_prefix('.') {
        return node.matches_class(class);
    }

    // 4. Attribute selector: "[name=val]" or "[disabled]"
    if selector.starts_with('[') && selector.ends_with(']') {
        let inner = &selector[1..selector.len() - 1].trim();
        if let Some((attr, val)) = inner.split_once('=') {
            let clean_val = val.trim().trim_matches(|c| c == '"' || c == '\'');
            return node.matches_attribute(attr.trim(), Some(clean_val));
        } else {
            return node.matches_attribute(inner, None);
        }
    }

    // 5. Tag selector: "button", "div", etc.
    node.matches_tag(selector)
}

/// Self-contained Chrome DevTools Protocol JSON-RPC 2.0 Server.
#[derive(Clone, Debug)]
pub struct MockCdpServer {
    document: VirtualDocument,
    viewport_width: u32,
    viewport_height: u32,
    device_scale_factor: f64,
    mobile: bool,
    enabled_domains: HashMap<String, bool>,
}

impl Default for MockCdpServer {
    fn default() -> Self {
        Self::new(1280, 720)
    }
}

impl MockCdpServer {
    pub fn new(viewport_width: u32, viewport_height: u32) -> Self {
        Self {
            document: VirtualDocument::new(viewport_width, viewport_height),
            viewport_width,
            viewport_height,
            device_scale_factor: 1.0,
            mobile: false,
            enabled_domains: HashMap::new(),
        }
    }

    pub fn document(&self) -> &VirtualDocument {
        &self.document
    }

    pub fn document_mut(&mut self) -> &mut VirtualDocument {
        &mut self.document
    }

    pub fn device_scale_factor(&self) -> f64 {
        self.device_scale_factor
    }

    pub fn is_mobile(&self) -> bool {
        self.mobile
    }

    /// Navigates the mock browser to the specified URL or loads HTML content.
    pub fn navigate(&mut self, url_or_html: &str) {
        if url_or_html.starts_with("data:text/html,") {
            let html = &url_or_html["data:text/html,".len()..];
            self.document.url = "data:text/html,...".to_string();
            self.document.load_html(html);
        } else if url_or_html.contains('<') && url_or_html.contains('>') {
            self.document.url = "http://localhost/mock-app".to_string();
            self.document.load_html(url_or_html);
        } else {
            self.document.url = url_or_html.to_string();
            let template = format!(
                "<html><head><title>Page: {url_or_html}</title></head><body><h1>Welcome to {url_or_html}</h1><div class=\"content\"><p>Ready for visual verification.</p></div></body></html>"
            );
            self.document.load_html(&template);
        }
    }

    /// Captures a screenshot of the current page rasterized onto the synthetic canvas as PNG bytes.
    pub fn capture_screenshot(&self) -> Vec<u8> {
        let mut canvas = SyntheticCanvas::new(self.viewport_width, self.viewport_height);
        self.document.render_to_canvas(&mut canvas);
        canvas.to_png_bytes()
    }

    /// Captures a screenshot and encodes it as Base64 string for CDP output.
    pub fn capture_screenshot_base64(&self) -> String {
        let mut canvas = SyntheticCanvas::new(self.viewport_width, self.viewport_height);
        self.document.render_to_canvas(&mut canvas);
        canvas.to_base64_png()
    }

    /// Executes CDP JSON-RPC 2.0 requests.
    pub fn handle_json_rpc(&mut self, request_json: &str) -> String {
        let req: Value = match serde_json::from_str(request_json) {
            Ok(v) => v,
            Err(e) => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("Parse error: {e}") }
                })
                .to_string();
            }
        };

        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = match req.get("method").and_then(|v| v.as_str()) {
            Some(m) => m,
            None => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32600, "message": "Invalid Request: missing method" }
                })
                .to_string();
            }
        };

        let params = req.get("params").cloned().unwrap_or(json!({}));
        let result = self.dispatch_method(method, &params);

        match result {
            Ok(val) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": val
            })
            .to_string(),
            Err(err) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": err.to_string() }
            })
            .to_string(),
        }
    }

    fn dispatch_method(&mut self, method: &str, params: &Value) -> CdpResult<Value> {
        match method {
            // --- Page Domain ---
            "Page.enable" => {
                self.enabled_domains.insert("Page".to_string(), true);
                Ok(json!({}))
            }
            "Page.navigate" => {
                let url = params
                    .get("url")
                    .and_then(|v| v.as_str())
                    .or_else(|| params.get("html").and_then(|v| v.as_str()))
                    .unwrap_or("about:blank");
                self.navigate(url);
                Ok(json!({
                    "frameId": "pandora-frame-1",
                    "loaderId": "pandora-loader-1"
                }))
            }
            "Page.captureScreenshot" => {
                let base64_png = self.capture_screenshot_base64();
                Ok(json!({
                    "data": base64_png
                }))
            }
            "Page.getLayoutMetrics" => Ok(json!({
                "layoutViewport": {
                    "pageX": 0,
                    "pageY": 0,
                    "clientWidth": self.viewport_width,
                    "clientHeight": self.viewport_height
                }
            })),

            // --- DOM Domain ---
            "DOM.enable" => {
                self.enabled_domains.insert("DOM".to_string(), true);
                Ok(json!({}))
            }
            "DOM.getDocument" => {
                let root_json = serialize_dom_tree(&self.document, self.document.root_id);
                Ok(json!({
                    "root": root_json
                }))
            }
            "DOM.querySelector" => {
                let selector = params
                    .get("selector")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CdpError::InvalidParameters("missing selector".into()))?;
                let node_id = self.document.query_selector(selector).unwrap_or(0);
                Ok(json!({
                    "nodeId": node_id
                }))
            }
            "DOM.querySelectorAll" => {
                let selector = params
                    .get("selector")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CdpError::InvalidParameters("missing selector".into()))?;
                let node_ids = self.document.query_selector_all(selector);
                Ok(json!({
                    "nodeIds": node_ids
                }))
            }
            "DOM.getOuterHTML" => {
                let node_id = params
                    .get("nodeId")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32)
                    .unwrap_or(self.document.root_id);
                let html = serialize_node_html(&self.document, node_id);
                Ok(json!({
                    "outerHTML": html
                }))
            }

            // --- Runtime Domain ---
            "Runtime.enable" => {
                self.enabled_domains.insert("Runtime".to_string(), true);
                Ok(json!({}))
            }
            "Runtime.evaluate" => {
                let expr = params
                    .get("expression")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CdpError::InvalidParameters("missing expression".into()))?;
                let val = self.document.evaluate_js(expr)?;
                let type_str = if val.is_string() {
                    "string"
                } else if val.is_number() {
                    "number"
                } else if val.is_boolean() {
                    "boolean"
                } else {
                    "object"
                };
                Ok(json!({
                    "result": {
                        "type": type_str,
                        "value": val
                    }
                }))
            }

            // --- Input Domain ---
            "Input.dispatchMouseEvent" => {
                let event_type = params
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("mousePressed");
                let x = params.get("x").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let y = params.get("y").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

                if event_type == "mousePressed" || event_type == "click" {
                    // Find topmost element containing (x, y)
                    let mut target_id = None;
                    for (&id, node) in &self.document.nodes {
                        if id != self.document.root_id && node.bounds.contains(x, y) {
                            target_id = Some(id);
                        }
                    }
                    if let Some(id) = target_id {
                        let _ = self.document.click(&format!("[data-id='{id}']"));
                    }
                }
                Ok(json!({}))
            }
            "Input.dispatchKeyEvent" => {
                let text = params
                    .get("text")
                    .and_then(|v| v.as_str())
                    .or_else(|| params.get("key").and_then(|v| v.as_str()))
                    .unwrap_or("");
                // Append text to currently focused element
                if let Some(focused_id) = self
                    .document
                    .nodes
                    .values()
                    .find(|n| n.is_focused)
                    .map(|n| n.id)
                {
                    if let Some(node) = self.document.nodes.get_mut(&focused_id) {
                        let mut current = node.attributes.get("value").cloned().unwrap_or_default();
                        current.push_str(text);
                        node.attributes.insert("value".to_string(), current);
                        node.text_content.push_str(text);
                    }
                }
                Ok(json!({}))
            }

            // --- Emulation Domain ---
            "Emulation.setDeviceMetricsOverride" => {
                let width = params
                    .get("width")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32)
                    .unwrap_or(self.viewport_width);
                let height = params
                    .get("height")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32)
                    .unwrap_or(self.viewport_height);
                if let Some(scale) = params.get("deviceScaleFactor").and_then(|v| v.as_f64()) {
                    self.device_scale_factor = scale;
                }
                if let Some(mobile) = params.get("mobile").and_then(|v| v.as_bool()) {
                    self.mobile = mobile;
                }
                self.viewport_width = width;
                self.viewport_height = height;
                self.document.viewport_width = width;
                self.document.viewport_height = height;
                self.document.compute_layout();
                Ok(json!({}))
            }

            unknown => Err(CdpError::MethodNotFound(unknown.to_string())),
        }
    }

    /// Executes autonomous visual verification steps (navigate, click, type, screenshot, assert)
    /// and generates a comprehensive verification report.
    pub fn execute_visual_steps(
        &mut self,
        steps: &[VisualStep],
    ) -> CdpResult<VisualVerificationReport> {
        let mut report = VisualVerificationReport {
            passed: true,
            steps_total: steps.len(),
            steps_passed: 0,
            assertions_passed: 0,
            assertions_failed: 0,
            screenshots: Vec::new(),
            errors: Vec::new(),
        };

        for step in steps {
            match step.step_type.as_str() {
                "navigate" => {
                    let url = step
                        .url
                        .as_deref()
                        .or(step.text.as_deref())
                        .unwrap_or("about:blank");
                    self.navigate(url);
                    report.steps_passed += 1;
                }
                "click" => {
                    let sel = step.selector.as_deref().unwrap_or("");
                    match self.document.click(sel) {
                        Ok(_) => report.steps_passed += 1,
                        Err(e) => {
                            report.passed = false;
                            report.errors.push(format!("click failed on '{sel}': {e}"));
                        }
                    }
                }
                "type" => {
                    let sel = step.selector.as_deref().unwrap_or("");
                    let text = step.text.as_deref().unwrap_or("");
                    match self.document.type_text(sel, text) {
                        Ok(_) => report.steps_passed += 1,
                        Err(e) => {
                            report.passed = false;
                            report
                                .errors
                                .push(format!("type failed on '{sel}' with text '{text}': {e}"));
                        }
                    }
                }
                "screenshot" => {
                    let name = step.name.clone().unwrap_or_else(|| "screenshot".to_string());
                    let mut canvas =
                        SyntheticCanvas::new(self.viewport_width, self.viewport_height);
                    self.document.render_to_canvas(&mut canvas);

                    let sha256 = canvas.sha256_digest();
                    let base64_png = canvas.to_base64_png();

                    // Check expected hash if specified
                    if let Some(expected) = &step.expected_hash {
                        if sha256.eq_ignore_ascii_case(expected) {
                            report.assertions_passed += 1;
                        } else {
                            report.assertions_failed += 1;
                            report.passed = false;
                            report.errors.push(format!(
                                "screenshot hash mismatch: expected {expected}, got {sha256}"
                            ));
                        }
                    }

                    report.screenshots.push(ScreenshotArtifact {
                        name,
                        width: self.viewport_width,
                        height: self.viewport_height,
                        sha256,
                        base64_png,
                    });
                    report.steps_passed += 1;
                }
                "assert_visible" => {
                    let sel = step.selector.as_deref().unwrap_or("");
                    if let Some(node_id) = self.document.query_selector(sel) {
                        if let Some(node) = self.document.nodes.get(&node_id) {
                            if node.is_visible {
                                report.assertions_passed += 1;
                                report.steps_passed += 1;
                            } else {
                                report.assertions_failed += 1;
                                report.passed = false;
                                report.errors.push(format!("element '{sel}' is not visible"));
                            }
                        }
                    } else {
                        report.assertions_failed += 1;
                        report.passed = false;
                        report
                            .errors
                            .push(format!("element '{sel}' not found in DOM"));
                    }
                }
                "assert_text" => {
                    let sel = step.selector.as_deref().unwrap_or("");
                    let expected = step
                        .expected_text
                        .as_deref()
                        .or(step.text.as_deref())
                        .unwrap_or("");
                    if let Some(node_id) = self.document.query_selector(sel) {
                        if let Some(node) = self.document.nodes.get(&node_id) {
                            if node.text_content.contains(expected) {
                                report.assertions_passed += 1;
                                report.steps_passed += 1;
                            } else {
                                report.assertions_failed += 1;
                                report.passed = false;
                                report.errors.push(format!(
                                    "text mismatch on '{sel}': expected to contain '{expected}', got '{}'",
                                    node.text_content
                                ));
                            }
                        }
                    } else {
                        report.assertions_failed += 1;
                        report.passed = false;
                        report
                            .errors
                            .push(format!("element '{sel}' not found in DOM"));
                    }
                }
                "assert_eval" => {
                    let expr = step.expression.as_deref().unwrap_or("");
                    match self.document.evaluate_js(expr) {
                        Ok(actual) => {
                            if let Some(expected) = &step.expected_value {
                                if &actual == expected {
                                    report.assertions_passed += 1;
                                    report.steps_passed += 1;
                                } else {
                                    report.assertions_failed += 1;
                                    report.passed = false;
                                    report.errors.push(format!(
                                        "eval assertion failed for '{expr}': expected {expected}, got {actual}"
                                    ));
                                }
                            } else if actual == json!(true) {
                                report.assertions_passed += 1;
                                report.steps_passed += 1;
                            } else {
                                report.assertions_failed += 1;
                                report.passed = false;
                                report.errors.push(format!(
                                    "eval assertion failed for '{expr}': got {actual}"
                                ));
                            }
                        }
                        Err(e) => {
                            report.assertions_failed += 1;
                            report.passed = false;
                            report.errors.push(format!("eval error for '{expr}': {e}"));
                        }
                    }
                }
                unknown => {
                    report.errors.push(format!("unknown visual step type '{unknown}'"));
                }
            }
        }

        Ok(report)
    }
}

fn serialize_dom_tree(doc: &VirtualDocument, node_id: u32) -> Value {
    if let Some(node) = doc.nodes.get(&node_id) {
        let child_values: Vec<Value> = node
            .children
            .iter()
            .map(|&cid| serialize_dom_tree(doc, cid))
            .collect();

        let mut attrs = Vec::new();
        for (k, v) in &node.attributes {
            attrs.push(k.clone());
            attrs.push(v.clone());
        }

        json!({
            "nodeId": node.id,
            "backendNodeId": node.id,
            "nodeType": if node.tag == "#document" { 9 } else { 1 },
            "nodeName": node.tag.to_uppercase(),
            "localName": node.tag,
            "nodeValue": node.text_content,
            "childNodeCount": child_values.len(),
            "children": child_values,
            "attributes": attrs
        })
    } else {
        Value::Null
    }
}

fn serialize_node_html(doc: &VirtualDocument, node_id: u32) -> String {
    if let Some(node) = doc.nodes.get(&node_id) {
        if node.tag == "#document" {
            let mut out = String::new();
            for &cid in &node.children {
                out.push_str(&serialize_node_html(doc, cid));
            }
            return out;
        }

        let mut out = format!("<{}", node.tag);
        for (k, v) in &node.attributes {
            out.push_str(&format!(" {k}=\"{v}\""));
        }
        if matches!(node.tag.as_str(), "input" | "img" | "br" | "hr") {
            out.push_str(" />");
            return out;
        }

        out.push('>');
        out.push_str(&node.text_content);
        for &cid in &node.children {
            out.push_str(&serialize_node_html(doc, cid));
        }
        out.push_str(&format!("</{}>", node.tag));
        out
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_dom_parsing_and_selectors() {
        let html = r#"
            <html>
                <head><title>Agent Pandora Visual Verification</title></head>
                <body>
                    <h1 id="title" class="main-heading">Dashboard</h1>
                    <div class="card">
                        <input id="user-input" type="text" placeholder="Enter username" />
                        <button id="login-btn" class="btn btn-primary">Login</button>
                    </div>
                </body>
            </html>
        "#;

        let mut doc = VirtualDocument::new(1280, 720);
        doc.load_html(html);

        assert_eq!(doc.title, "Agent Pandora Visual Verification");

        // Test selectors
        let h1_id = doc.query_selector("#title").expect("h1#title found");
        assert_eq!(doc.nodes.get(&h1_id).unwrap().text_content, "Dashboard");

        let btn_id = doc.query_selector(".btn-primary").expect("btn found");
        assert_eq!(doc.nodes.get(&btn_id).unwrap().text_content, "Login");

        let input_id = doc.query_selector("input#user-input").expect("input found");
        assert_eq!(
            doc.nodes
                .get(&input_id)
                .unwrap()
                .get_attribute("placeholder"),
            Some("Enter username")
        );

        // Interaction test
        doc.click("#login-btn").unwrap();
        assert!(doc.nodes.get(&btn_id).unwrap().is_focused);
        assert_eq!(doc.nodes.get(&btn_id).unwrap().click_count, 1);

        doc.type_text("#user-input", "pandora_admin").unwrap();
        assert_eq!(
            doc.nodes.get(&input_id).unwrap().get_attribute("value"),
            Some("pandora_admin")
        );

        // JS evaluation test
        assert_eq!(
            doc.evaluate_js("document.title").unwrap(),
            json!("Agent Pandora Visual Verification")
        );
        assert_eq!(
            doc.evaluate_js("document.querySelector('#user-input').value")
                .unwrap(),
            json!("pandora_admin")
        );
    }

    #[test]
    fn cdp_json_rpc_page_and_dom_methods() {
        let mut server = MockCdpServer::new(1024, 768);

        // 1. Page.enable
        let res = server.handle_json_rpc(r#"{"id":1,"method":"Page.enable"}"#);
        assert!(res.contains(r#""id":1"#));
        assert!(res.contains(r#""result":{}"#));

        // 2. Page.navigate
        let nav_res = server.handle_json_rpc(
            r#"{"id":2,"method":"Page.navigate","params":{"html":"<h1>Visual Test</h1><button id=\"test-btn\">Click Me</button>"}}"#,
        );
        assert!(nav_res.contains("pandora-frame-1"));

        // 3. DOM.querySelector
        let sel_res = server.handle_json_rpc(
            r##"{"id":3,"method":"DOM.querySelector","params":{"selector":"#test-btn"}}"##,
        );
        assert!(sel_res.contains(r#""nodeId""#));

        // 4. Page.captureScreenshot
        let ss_res = server.handle_json_rpc(r#"{"id":4,"method":"Page.captureScreenshot"}"#);
        assert!(ss_res.contains(r#""data":""#));

        // 5. Runtime.evaluate
        let eval_res = server.handle_json_rpc(
            r##"{"id":5,"method":"Runtime.evaluate","params":{"expression":"document.querySelector('#test-btn').textContent"}}"##,
        );
        assert!(eval_res.contains("Click Me"));
    }

    #[test]
    fn automated_visual_steps_execution() {
        let mut server = MockCdpServer::new(1280, 720);

        let steps = vec![
            VisualStep {
                step_type: "navigate".into(),
                url: Some("<h1>Automated Testing</h1><button id=\"action-btn\">Action</button>".into()),
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
                step_type: "assert_visible".into(),
                url: None,
                selector: Some("#action-btn".into()),
                text: None,
                name: None,
                expression: None,
                expected_hash: None,
                expected_text: None,
                expected_value: None,
                tolerance_percent: None,
            },
            VisualStep {
                step_type: "assert_text".into(),
                url: None,
                selector: Some("h1".into()),
                text: Some("Automated Testing".into()),
                name: None,
                expression: None,
                expected_hash: None,
                expected_text: Some("Automated Testing".into()),
                expected_value: None,
                tolerance_percent: None,
            },
            VisualStep {
                step_type: "click".into(),
                url: None,
                selector: Some("#action-btn".into()),
                text: None,
                name: None,
                expression: None,
                expected_hash: None,
                expected_text: None,
                expected_value: None,
                tolerance_percent: None,
            },
            VisualStep {
                step_type: "screenshot".into(),
                url: None,
                selector: None,
                text: None,
                name: Some("test_dashboard".into()),
                expression: None,
                expected_hash: None,
                expected_text: None,
                expected_value: None,
                tolerance_percent: None,
            },
        ];

        let report = server.execute_visual_steps(&steps).expect("steps executed");
        assert!(report.passed);
        assert_eq!(report.steps_total, 5);
        assert_eq!(report.assertions_passed, 2);
        assert_eq!(report.screenshots.len(), 1);
        assert_eq!(report.screenshots[0].name, "test_dashboard");
        assert_eq!(report.screenshots[0].sha256.len(), 64);
    }
}
