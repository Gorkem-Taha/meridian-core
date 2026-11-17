//! The dependency graph over a document's resources, and its deterministic
//! ordering.
//!
//! Two edge kinds are kept structurally distinct because they are not the same
//! concept: `dependsOn` is a hard ordering constraint, while `notifies` is a
//! conditional, deferred trigger that says nothing about order.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::fmt;

use meridian_ir::{Document, IfMissing, Position, Resource};

/// Distinguishes ordering constraints from deferred triggers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// From `dependsOn`: `from` must be applied before `to`.
    HardOrder,
    /// From `notifies`: `to` is triggered if `from` reports a change. It
    /// constrains nothing about apply order.
    Notify,
}

impl fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EdgeKind::HardOrder => "dependsOn",
            EdgeKind::Notify => "notifies",
        })
    }
}

/// One resource in the graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    /// The resource's declaration order in the source document. It is the only
    /// tiebreak used by the topological sort.
    pub index: usize,
    pub resource: Resource,
}

/// A directed relation between two resources. The same pair may carry both a
/// `HardOrder` and a `Notify` edge; they are stored separately and never
/// collapsed.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    /// The notify action (for example "restart"); empty for `HardOrder`.
    pub action: String,
    /// The dangling-edge policy declared on the source edge.
    pub if_missing: IfMissing,
    pub pos: Position,
}

/// A graph-construction or sort failure.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Error {
    pub msg: String,
    pub pos: Position,
    /// For a dependency cycle, the ids along it, closing on the id it opened
    /// with. Empty otherwise.
    pub cycle: Vec<String>,
}

impl Error {
    fn at(pos: &Position, msg: String) -> Self {
        Error {
            msg,
            pos: pos.clone(),
            cycle: Vec::new(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.pos.line > 0 || !self.pos.file.is_empty() {
            write!(f, "{}: ", self.pos)?;
        }
        f.write_str(&self.msg)?;
        if !self.cycle.is_empty() {
            write!(f, ": {}", self.cycle.join(" -> "))?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

/// A built, validated resource graph.
#[derive(Clone, Debug)]
pub struct Graph {
    nodes: Vec<Node>,
    by_id: HashMap<String, usize>,
    edges: Vec<Edge>,
    /// Per node, the indices of its outgoing and incoming `HardOrder` edges.
    out_hard: Vec<Vec<usize>>,
    in_hard: Vec<Vec<usize>>,
}

impl Graph {
    /// Builds the graph for one IR document.
    pub fn from_document(doc: &Document) -> Result<Graph, Error> {
        Graph::build(doc.resources())
    }

    /// Builds the graph from resources in declaration order.
    ///
    /// Edges pointing at resources that are not present are an error here:
    /// pruning a resource is the condition stage's job, and it applies each
    /// edge's `ifMissing` policy before the graph is built.
    pub fn build(resources: &[Resource]) -> Result<Graph, Error> {
        let mut g = Graph {
            nodes: Vec::with_capacity(resources.len()),
            by_id: HashMap::with_capacity(resources.len()),
            edges: Vec::new(),
            out_hard: vec![Vec::new(); resources.len()],
            in_hard: vec![Vec::new(); resources.len()],
        };
        for (i, r) in resources.iter().enumerate() {
            if r.id.is_empty() {
                return Err(Error::at(
                    &r.pos,
                    format!("resource at declaration index {i} has no id"),
                ));
            }
            // The index comes from declaration order here, not from the parsed
            // field, so a graph built from hand-made resources still sorts stably.
            if g.by_id.insert(r.id.clone(), i).is_some() {
                return Err(Error::at(
                    &r.pos,
                    format!("duplicate resource id {:?}", r.id),
                ));
            }
            g.nodes.push(Node {
                id: r.id.clone(),
                index: i,
                resource: r.clone(),
            });
        }

        for (to, r) in resources.iter().enumerate() {
            for dep in &r.depends_on {
                if dep.resource == r.id {
                    return Err(Error::at(
                        &dep.pos,
                        format!("resource {:?} depends on itself", r.id),
                    ));
                }
                let Some(&from) = g.by_id.get(&dep.resource) else {
                    return Err(Error::at(
                        &dep.pos,
                        format!(
                            "resource {:?} depends on unknown resource {:?}",
                            r.id, dep.resource
                        ),
                    ));
                };
                let idx = g.edges.len();
                g.edges.push(Edge {
                    from: dep.resource.clone(),
                    to: r.id.clone(),
                    kind: EdgeKind::HardOrder,
                    action: String::new(),
                    if_missing: dep.if_missing.clone(),
                    pos: dep.pos.clone(),
                });
                g.out_hard[from].push(idx);
                g.in_hard[to].push(idx);
            }
            for n in &r.notifies {
                if n.resource == r.id {
                    return Err(Error::at(
                        &n.pos,
                        format!("resource {:?} notifies itself", r.id),
                    ));
                }
                if !g.by_id.contains_key(&n.resource) {
                    return Err(Error::at(
                        &n.pos,
                        format!(
                            "resource {:?} notifies unknown resource {:?}",
                            r.id, n.resource
                        ),
                    ));
                }
                g.edges.push(Edge {
                    from: r.id.clone(),
                    to: n.resource.clone(),
                    kind: EdgeKind::Notify,
                    action: n.action.clone(),
                    if_missing: n.if_missing.clone(),
                    pos: n.pos.clone(),
                });
            }
        }
        Ok(g)
    }

    /// The graph's nodes in declaration order.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Every edge, in the order they were declared.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Only the edges of the given kind, in declaration order.
    pub fn edges_of_kind(&self, kind: EdgeKind) -> impl Iterator<Item = &Edge> {
        self.edges.iter().filter(move |e| e.kind == kind)
    }

    /// Every edge from one resource to another. A `dependsOn` and a `notifies`
    /// between the same pair both appear, distinctly.
    pub fn edges_between<'a>(
        &'a self,
        from: &'a str,
        to: &'a str,
    ) -> impl Iterator<Item = &'a Edge> {
        self.edges
            .iter()
            .filter(move |e| e.from == from && e.to == to)
    }

    /// The node with the given id.
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.by_id.get(id).map(|&i| &self.nodes[i])
    }

    /// The ids a resource must be applied after, in declaration order.
    pub fn dependencies(&self, id: &str) -> Vec<&str> {
        self.neighbours(id, &self.in_hard, |e| &e.from)
    }

    /// The ids that must be applied after a resource, in declaration order.
    pub fn dependents(&self, id: &str) -> Vec<&str> {
        self.neighbours(id, &self.out_hard, |e| &e.to)
    }

    fn neighbours(
        &self,
        id: &str,
        adjacency: &[Vec<usize>],
        pick: fn(&Edge) -> &String,
    ) -> Vec<&str> {
        let Some(&node) = self.by_id.get(id) else {
            return Vec::new();
        };
        let mut indices: Vec<usize> = adjacency[node]
            .iter()
            .map(|&e| self.by_id[pick(&self.edges[e])])
            .collect();
        indices.sort_unstable();
        indices.dedup();
        indices
            .into_iter()
            .map(|i| self.nodes[i].id.as_str())
            .collect()
    }

    fn target(&self, edge: usize) -> usize {
        self.by_id[&self.edges[edge].to]
    }

    /// The resource ids in an order that satisfies every `dependsOn` edge.
    ///
    /// Ties between independent resources are broken by declaration index, so
    /// the same graph always produces the same order. Notify edges are
    /// deliberately excluded: `notifies` is a deferred trigger, not an ordering
    /// constraint, and a notification loop is legal where a dependency loop is
    /// not.
    pub fn topo_sort(&self) -> Result<Vec<&str>, Error> {
        Ok(self
            .topo_sort_nodes()?
            .into_iter()
            .map(|n| n.id.as_str())
            .collect())
    }

    /// [`Graph::topo_sort`] returning full nodes instead of ids.
    pub fn topo_sort_nodes(&self) -> Result<Vec<&Node>, Error> {
        let mut indegree: Vec<usize> = self.in_hard.iter().map(Vec::len).collect();
        // A min-heap of declaration indices: using the index, not the id, keeps
        // the tiebreak tied to source order instead of to string sorting.
        let mut ready: BinaryHeap<Reverse<usize>> = (0..self.nodes.len())
            .filter(|&i| indegree[i] == 0)
            .map(Reverse)
            .collect();

        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(Reverse(i)) = ready.pop() {
            order.push(&self.nodes[i]);
            for &edge in &self.out_hard[i] {
                let to = self.target(edge);
                indegree[to] -= 1;
                if indegree[to] == 0 {
                    ready.push(Reverse(to));
                }
            }
        }

        if order.len() != self.nodes.len() {
            return Err(self.cycle_error(&indegree));
        }
        Ok(order)
    }

    /// Finds one concrete cycle among the nodes the sort could not place and
    /// reports its full path, so the author sees which resources are involved
    /// and not only that a cycle exists.
    fn cycle_error(&self, indegree: &[usize]) -> Error {
        #[derive(Clone, Copy, PartialEq)]
        enum State {
            Unvisited,
            OnPath,
            Done,
        }
        let remaining = |i: usize| indegree[i] > 0;
        let mut state = vec![State::Unvisited; self.nodes.len()];

        // Walk in declaration order so the reported cycle is itself
        // deterministic. The search keeps its own stack: a dependency chain can
        // be as long as the document, and recursion would be bounded by the
        // thread's stack instead.
        for start in 0..self.nodes.len() {
            if !remaining(start) || state[start] != State::Unvisited {
                continue;
            }
            // Each entry is a node on the current path and how many of its
            // outgoing edges have been followed so far.
            let mut path: Vec<(usize, usize)> = vec![(start, 0)];
            state[start] = State::OnPath;
            while let Some(&mut (node, ref mut followed)) = path.last_mut() {
                let Some(&edge) = self.out_hard[node].get(*followed) else {
                    state[node] = State::Done;
                    path.pop();
                    continue;
                };
                *followed += 1;
                let next = self.target(edge);
                if !remaining(next) {
                    continue;
                }
                match state[next] {
                    State::OnPath => {
                        // Trim the path back to where the cycle opens, then close it.
                        let opens = path.iter().position(|&(n, _)| n == next).unwrap_or(0);
                        let mut cycle: Vec<String> = path[opens..]
                            .iter()
                            .map(|&(n, _)| self.nodes[n].id.clone())
                            .collect();
                        cycle.push(self.nodes[next].id.clone());
                        return Error {
                            msg: format!("dependency cycle among {} resources", cycle.len() - 1),
                            pos: self.nodes[next].resource.pos.clone(),
                            cycle,
                        };
                    }
                    State::Unvisited => {
                        state[next] = State::OnPath;
                        path.push((next, 0));
                    }
                    State::Done => {}
                }
            }
        }
        Error {
            msg: "dependency cycle detected but could not be traced".to_owned(),
            ..Error::default()
        }
    }
}
