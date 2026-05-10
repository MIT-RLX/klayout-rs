//! Timing graph: pins as nodes, timing arcs + interconnect as edges.

use smol_str::SmolStr;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

#[derive(Clone, Debug)]
pub struct Node {
    pub id: NodeId,
    /// Hierarchical name `instance/pin`. Top-level primary pins use
    /// `(input)/<name>` or `(output)/<name>`.
    pub name: SmolStr,
    pub kind: NodeKind,
    /// Output capacitance load on this pin, used for delay scaling.
    /// Aggregates fanout pin caps + interconnect ground caps.
    pub load_cap: f64,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    PrimaryInput,
    PrimaryOutput,
    /// An input pin of an internal cell (combinational data input).
    CellInput,
    /// An output pin of an internal cell.
    CellOutput,
    /// A clock pin (input pin of a sequential element).
    Clock,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    /// Timing arc inside a cell (from CellInput → CellOutput, e.g.
    /// `A → Y` in an inverter).
    CellArc,
    /// Net interconnect between a driver (CellOutput / PrimaryInput)
    /// and one or more sinks (CellInput / PrimaryOutput).
    Net,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    /// Edge delay in time-units (matches Liberty / SPEF units).
    pub delay: f64,
}

#[derive(Default, Clone, Debug)]
pub struct TimingGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// CSR-style adjacency: `edges_from[i]` is the list of edge indices
    /// whose `from == NodeId(i)`. Computed lazily by `finalize()`.
    pub edges_from: Vec<Vec<u32>>,
    pub edges_to: Vec<Vec<u32>>,
}

impl TimingGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, name: impl Into<SmolStr>, kind: NodeKind) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node {
            id,
            name: name.into(),
            kind,
            load_cap: 0.0,
        });
        id
    }

    pub fn add_edge(&mut self, from: NodeId, to: NodeId, kind: EdgeKind, delay: f64) {
        self.edges.push(Edge {
            from,
            to,
            kind,
            delay,
        });
    }

    pub fn finalize(&mut self) {
        let n = self.nodes.len();
        self.edges_from = vec![Vec::new(); n];
        self.edges_to = vec![Vec::new(); n];
        for (i, e) in self.edges.iter().enumerate() {
            self.edges_from[e.from.0 as usize].push(i as u32);
            self.edges_to[e.to.0 as usize].push(i as u32);
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    pub fn primary_inputs(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::PrimaryInput | NodeKind::Clock))
            .map(|n| n.id)
    }

    pub fn primary_outputs(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.nodes
            .iter()
            .filter(|n| n.kind == NodeKind::PrimaryOutput)
            .map(|n| n.id)
    }

    pub fn outgoing(&self, n: NodeId) -> impl Iterator<Item = &Edge> + '_ {
        self.edges_from[n.0 as usize]
            .iter()
            .map(move |&i| &self.edges[i as usize])
    }

    pub fn incoming(&self, n: NodeId) -> impl Iterator<Item = &Edge> + '_ {
        self.edges_to[n.0 as usize]
            .iter()
            .map(move |&i| &self.edges[i as usize])
    }
}
