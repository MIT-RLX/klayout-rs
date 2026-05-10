//! STA report generation: slack tables and worst-path traces.

use crate::graph::{NodeId, TimingGraph};
use std::fmt::Write as _;

#[derive(Clone, Debug)]
pub struct SlackReport {
    pub entries: Vec<SlackEntry>,
}

#[derive(Clone, Debug)]
pub struct SlackEntry {
    pub node: NodeId,
    pub name: smol_str::SmolStr,
    pub arrival: f64,
    pub required: f64,
    pub slack: f64,
}

impl SlackReport {
    pub fn build(g: &TimingGraph, arrivals: &[f64], requireds: &[f64]) -> Self {
        let mut entries: Vec<SlackEntry> = g
            .nodes
            .iter()
            .map(|n| {
                let i = n.id.0 as usize;
                SlackEntry {
                    node: n.id,
                    name: n.name.clone(),
                    arrival: arrivals[i],
                    required: requireds[i],
                    slack: requireds[i] - arrivals[i],
                }
            })
            .collect();
        entries.sort_by(|a, b| a.slack.partial_cmp(&b.slack).unwrap_or(std::cmp::Ordering::Equal));
        SlackReport { entries }
    }

    /// Worst (most negative) slack across all nodes.
    pub fn worst_slack(&self) -> Option<f64> {
        self.entries.first().map(|e| e.slack)
    }

    /// Top-N worst-slack entries.
    pub fn worst_n(&self, n: usize) -> &[SlackEntry] {
        let n = n.min(self.entries.len());
        &self.entries[..n]
    }
}

/// Trace a worst-arrival path from primary inputs to `endpoint`.
/// Walks backward, at each step picking the predecessor that
/// contributed the maximum arrival.
pub fn trace_worst_path(
    g: &TimingGraph,
    arrivals: &[f64],
    endpoint: NodeId,
) -> Vec<NodeId> {
    let mut path = vec![endpoint];
    let mut cur = endpoint;
    loop {
        let target = arrivals[cur.0 as usize];
        let mut best: Option<(NodeId, f64)> = None;
        for e in g.incoming(cur) {
            let cand = arrivals[e.from.0 as usize] + e.delay;
            if (cand - target).abs() < 1e-9 {
                let from_arr = arrivals[e.from.0 as usize];
                match best {
                    None => best = Some((e.from, from_arr)),
                    Some((_, prev)) if from_arr > prev => best = Some((e.from, from_arr)),
                    _ => {}
                }
            }
        }
        match best {
            None => break,
            Some((p, _)) => {
                path.push(p);
                cur = p;
            }
        }
    }
    path.reverse();
    path
}

#[derive(Clone, Debug)]
pub struct PathReport {
    pub start: NodeId,
    pub end: NodeId,
    pub arrival: f64,
    pub required: f64,
    pub slack: f64,
    pub nodes: Vec<NodeId>,
    pub edge_delays: Vec<f64>,
}

pub fn format_path_report(g: &TimingGraph, arrivals: &[f64], requireds: &[f64], path: &[NodeId]) -> String {
    let mut out = String::new();
    if path.is_empty() {
        return out;
    }
    // path.len() ≥ 1 here, so path[0] and path[len-1] are in bounds.
    let first = path[0];
    let last = path[path.len() - 1];
    let _ = writeln!(
        out,
        "Path: {} → {}",
        g.node(first).name,
        g.node(last).name
    );
    let _ = writeln!(out, "  arrival = {:.4}", arrivals[last.0 as usize]);
    let _ = writeln!(out, "  required = {:.4}", requireds[last.0 as usize]);
    let _ = writeln!(
        out,
        "  slack = {:.4}",
        requireds[last.0 as usize] - arrivals[last.0 as usize]
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "  step  cumul    delta   pin");
    let mut prev = arrivals[first.0 as usize];
    for (i, nid) in path.iter().enumerate() {
        let now = arrivals[nid.0 as usize];
        let delta = now - prev;
        let _ = writeln!(
            out,
            "  {:>4}  {:>7.4}  {:>+7.4}  {}",
            i,
            now,
            delta,
            g.node(*nid).name
        );
        prev = now;
    }
    out
}
