#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// A node in the evolutionary lineage DAG representing a specific organism / genome candidate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvolutionLineageNode {
    pub id: String,
    pub parent_id: Option<String>,
    pub generation: u32,
    pub score: f64,
    pub latency_ms: u64,
    pub model: String,
    pub mutation_summary: String,
    pub failure_case_addressed: Option<String>,
    pub is_champion: bool,
}

/// A directed edge connecting a parent organism to its child mutation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvolutionLineageEdge {
    pub from: String,
    pub to: String,
    pub score_delta: f64,
    pub mutation_type: String,
}

/// The evolutionary lineage DAG capturing all evaluated variants over generations.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EvolutionLineage {
    pub nodes: Vec<EvolutionLineageNode>,
    pub edges: Vec<EvolutionLineageEdge>,
}

const VISUALIZER_HTML_TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>__TITLE__ - Evolutionary Lineage Visualizer</title>
  <style>
    body {
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, monospace;
      background: #0f172a;
      color: #e2e8f0;
      margin: 0;
      padding: 24px;
    }
    h1 { color: #38bdf8; margin-bottom: 8px; }
    .subtitle { color: #94a3b8; font-size: 14px; margin-bottom: 24px; }
    .grid { display: grid; grid-template-columns: 1fr 340px; gap: 24px; }
    #graph-container {
      background: #1e293b;
      border: 1px solid #334155;
      border-radius: 8px;
      padding: 16px;
      min-height: 500px;
      overflow: auto;
    }
    #details-panel {
      background: #1e293b;
      border: 1px solid #334155;
      border-radius: 8px;
      padding: 16px;
      font-size: 13px;
    }
    .node-card {
      background: #334155;
      border-radius: 6px;
      padding: 10px;
      margin-bottom: 12px;
      cursor: pointer;
      border-left: 4px solid #64748b;
    }
    .node-card:hover { background: #475569; }
    .node-card.champion { border-left-color: #22c55e; }
    .score { font-weight: bold; color: #38bdf8; }
  </style>
</head>
<body>
  <h1>__TITLE__</h1>
  <div class="subtitle">Darwinian Lineage DAG &amp; Fitness Evolution</div>
  <div class="grid">
    <div id="graph-container">
      <svg id="lineage-svg" width="800" height="600"></svg>
    </div>
    <div id="details-panel">
      <h3>Organism Inspector</h3>
      <div id="inspector-content">Select a node in the graph to inspect its genome and mutations.</div>
    </div>
  </div>
  <script>
    const nodes = __NODES_JSON__;
    const edges = __EDGES_JSON__;
    const svg = document.getElementById("lineage-svg");
    const inspector = document.getElementById("inspector-content");

    const genMap = {};
    nodes.forEach(n => {
      if (!genMap[n.generation]) genMap[n.generation] = [];
      genMap[n.generation].push(n);
    });

    const genKeys = Object.keys(genMap).map(Number).sort((a,b)=>a-b);
    const colWidth = 180;
    const rowHeight = 90;
    const positions = {};

    genKeys.forEach((g, colIdx) => {
      genMap[g].forEach((node, rowIdx) => {
        const x = 60 + colIdx * colWidth;
        const y = 60 + rowIdx * rowHeight;
        positions[node.id] = { x, y };
      });
    });

    edges.forEach(e => {
      const p1 = positions[e.from];
      const p2 = positions[e.to];
      if (p1 && p2) {
        const line = document.createElementNS("http://www.w3.org/2000/svg", "line");
        line.setAttribute("x1", p1.x + 50);
        line.setAttribute("y1", p1.y);
        line.setAttribute("x2", p2.x - 50);
        line.setAttribute("y2", p2.y);
        line.setAttribute("stroke", e.score_delta >= 0 ? "#22c55e" : "#ef4444");
        line.setAttribute("stroke-width", "2");
        line.setAttribute("stroke-dasharray", e.score_delta >= 0 ? "none" : "4");
        svg.appendChild(line);
      }
    });

    nodes.forEach(n => {
      const pos = positions[n.id];
      if (!pos) return;
      const g = document.createElementNS("http://www.w3.org/2000/svg", "g");
      g.style.cursor = "pointer";
      g.onclick = () => {
        inspector.innerHTML = `
          <h4>${n.id} ${n.is_champion ? '<span style="color:#22c55e">[CHAMPION]</span>' : ''}</h4>
          <p><b>Generation:</b> ${n.generation}</p>
          <p><b>Score:</b> <span class="score">${n.score.toFixed(3)}</span></p>
          <p><b>Latency:</b> ${n.latency_ms} ms</p>
          <p><b>Model:</b> ${n.model}</p>
          <p><b>Mutation:</b> ${n.mutation_summary}</p>
          <p><b>Failure Addressed:</b> ${n.failure_case_addressed || 'None'}</p>
          <p><b>Parent:</b> ${n.parent_id || 'Root'}</p>
        `;
      };

      const rect = document.createElementNS("http://www.w3.org/2000/svg", "rect");
      rect.setAttribute("x", pos.x - 55);
      rect.setAttribute("y", pos.y - 25);
      rect.setAttribute("width", 110);
      rect.setAttribute("height", 50);
      rect.setAttribute("rx", 6);
      rect.setAttribute("fill", n.is_champion ? "#14532d" : "#1e293b");
      rect.setAttribute("stroke", n.is_champion ? "#22c55e" : "#38bdf8");
      rect.setAttribute("stroke-width", "2");

      const text = document.createElementNS("http://www.w3.org/2000/svg", "text");
      text.setAttribute("x", pos.x);
      text.setAttribute("y", pos.y);
      text.setAttribute("fill", "#f8fafc");
      text.setAttribute("font-size", "11px");
      text.setAttribute("text-anchor", "middle");
      text.textContent = n.id;

      const scoreText = document.createElementNS("http://www.w3.org/2000/svg", "text");
      scoreText.setAttribute("x", pos.x);
      scoreText.setAttribute("y", pos.y + 16);
      scoreText.setAttribute("fill", "#38bdf8");
      scoreText.setAttribute("font-size", "10px");
      scoreText.setAttribute("text-anchor", "middle");
      scoreText.textContent = `Score: ${n.score.toFixed(2)}`;

      g.appendChild(rect);
      g.appendChild(text);
      g.appendChild(scoreText);
      svg.appendChild(g);
    });
  </script>
</body>
</html>"##;

impl EvolutionLineage {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    pub fn record_node(&mut self, node: EvolutionLineageNode) {
        if let Some(parent) = node
            .parent_id
            .as_ref()
            .and_then(|parent_id| self.nodes.iter().find(|n| &n.id == parent_id))
        {
            let delta = node.score - parent.score;
            self.edges.push(EvolutionLineageEdge {
                from: parent.id.clone(),
                to: node.id.clone(),
                score_delta: delta,
                mutation_type: node.mutation_summary.clone(),
            });
        }
        self.nodes.push(node);
    }

    pub fn champion(&self) -> Option<&EvolutionLineageNode> {
        self.nodes.iter().find(|n| n.is_champion)
    }

    /// Renders an ASCII text tree of the lineage DAG.
    pub fn render_ascii_tree(&self) -> String {
        let mut out = String::new();
        out.push_str("Evolutionary Lineage DAG:\n");
        for node in &self.nodes {
            let prefix = if node.is_champion {
                "  * [CHAMPION] "
            } else {
                "  - "
            };
            let parent_str = node
                .parent_id
                .as_deref()
                .map(|p| format!(" (parent: {p})"))
                .unwrap_or_default();
            out.push_str(&format!(
                "{}{}: gen={}, score={:.3}, latency={}ms, model={}, mut=\"{}\"{}\n",
                prefix,
                node.id,
                node.generation,
                node.score,
                node.latency_ms,
                node.model,
                node.mutation_summary,
                parent_str
            ));
        }
        out
    }

    /// Generates a standalone, dependency-free interactive HTML lineage visualizer
    /// with an SVG DAG, node tooltips, and generation timeline.
    pub fn export_html_visualizer(&self, title: &str) -> String {
        let nodes_json = serde_json::to_string(&self.nodes).unwrap_or_else(|_| "[]".to_string());
        let edges_json = serde_json::to_string(&self.edges).unwrap_or_else(|_| "[]".to_string());

        VISUALIZER_HTML_TEMPLATE
            .replace("__TITLE__", title)
            .replace("__NODES_JSON__", &nodes_json)
            .replace("__EDGES_JSON__", &edges_json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lineage_recording_and_ascii() {
        let mut lineage = EvolutionLineage::new();
        lineage.record_node(EvolutionLineageNode {
            id: "org-0".to_string(),
            parent_id: None,
            generation: 0,
            score: 0.72,
            latency_ms: 1200,
            model: "claude-3-7-sonnet".to_string(),
            mutation_summary: "Initial root organism".to_string(),
            failure_case_addressed: None,
            is_champion: false,
        });
        lineage.record_node(EvolutionLineageNode {
            id: "org-1".to_string(),
            parent_id: Some("org-0".to_string()),
            generation: 1,
            score: 0.88,
            latency_ms: 950,
            model: "claude-3-7-sonnet".to_string(),
            mutation_summary: "Added regex validation pre-check".to_string(),
            failure_case_addressed: Some("Input malformed error".to_string()),
            is_champion: true,
        });

        assert_eq!(lineage.edges.len(), 1);
        assert_eq!(lineage.edges[0].from, "org-0");
        assert_eq!(lineage.edges[0].to, "org-1");
        assert!((lineage.edges[0].score_delta - 0.16).abs() < 1e-4);

        let ascii = lineage.render_ascii_tree();
        assert!(ascii.contains("[CHAMPION] org-1"));

        let html = lineage.export_html_visualizer("Test Lineage");
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("org-1"));
        assert!(html.contains("Test Lineage"));
    }
}
