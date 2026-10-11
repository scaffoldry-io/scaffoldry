//! The calculation graph of an app: which computed field reads which.
//!
//! A node is `(table, field)`. An edge runs from what is read to what reads it. A formula reads the
//! fields named in `{...}`. A lookup, rollup, or count reads the link it goes through and, on the other
//! table, the field it shows. The graph has no cycle, or the manifest does not validate.

use crate::{AppManifest, FieldSpec, FieldType};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId {
    pub table: String,
    pub field: String,
}

/// A cycle, named in words: `Status depends on Remaining, which depends on Status.`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleError {
    pub path: Vec<NodeId>,
    text: String,
}

impl std::fmt::Display for CycleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::error::Error for CycleError {}

#[derive(Debug, Clone)]
pub struct Graph {
    /// Every node, in a run order: a node comes after everything it reads.
    order: Vec<NodeId>,
    /// What each node is read by.
    read_by: HashMap<NodeId, Vec<NodeId>>,
}

fn field_of<'a>(m: &'a AppManifest, table: &str, name: &str) -> Option<&'a FieldSpec> {
    m.tables.iter().find(|t| t.id == table).and_then(|t| t.fields.iter().find(|f| f.name == name))
}

/// The nodes a computed field reads.
fn reads(m: &AppManifest, table: &str, f: &FieldSpec) -> Vec<NodeId> {
    let here = |name: &str| NodeId { table: table.to_string(), field: name.to_string() };
    let mut out = Vec::new();
    match f.field_type {
        FieldType::Formula => {
            let refs = f.formula_expression.as_deref().and_then(crate::formula_field_refs).unwrap_or_default();
            out.extend(refs.iter().filter(|r| field_of(m, table, r).is_some()).map(|r| here(r)));
        }
        FieldType::Lookup | FieldType::Rollup | FieldType::Count => {
            if let Some(via) = f.computed_via.as_deref() {
                if field_of(m, table, via).is_some() {
                    out.push(here(via));
                }
                let target_table = field_of(m, table, via).and_then(|v| v.link_target_table.as_deref());
                if let (Some(target_table), Some(target)) = (target_table, f.target_display_field.as_deref()) {
                    if field_of(m, target_table, target).is_some() {
                        out.push(NodeId { table: target_table.to_string(), field: target.to_string() });
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn label(m: &AppManifest, n: &NodeId, with_table: bool) -> String {
    let field = field_of(m, &n.table, &n.field).map(|f| if f.label.is_empty() { f.name.clone() } else { f.label.clone() }).unwrap_or_else(|| n.field.clone());
    if with_table {
        let table = m.tables.iter().find(|t| t.id == n.table).map(|t| if t.name.is_empty() { t.id.clone() } else { t.name.clone() }).unwrap_or_else(|| n.table.clone());
        format!("{field} of {table}")
    } else {
        field
    }
}

/// The plain sentence for a cycle that starts and ends at `path[0]`.
fn sentence(m: &AppManifest, path: &[NodeId]) -> String {
    let across = path.iter().any(|n| n.table != path[0].table);
    let names: Vec<String> = path.iter().chain(path.first()).map(|n| label(m, n, across)).collect();
    let mut s = format!("{} depends on {}", names[0], names[1]);
    for n in &names[2..] {
        s.push_str(&format!(", which depends on {n}"));
    }
    s.push('.');
    s
}

/// The graph of the manifest, or the first cycle in it.
pub fn build_graph(m: &AppManifest) -> Result<Graph, CycleError> {
    // A field name used twice is one node. The validator reports the duplicate.
    let mut seen_nodes: HashSet<NodeId> = HashSet::new();
    let nodes: Vec<NodeId> = m
        .tables
        .iter()
        .flat_map(|t| t.fields.iter().map(move |f| NodeId { table: t.id.clone(), field: f.name.clone() }))
        .filter(|n| seen_nodes.insert(n.clone()))
        .collect();
    let known: HashSet<&NodeId> = nodes.iter().collect();
    // `reads_of[n]` is what n reads. `read_by[n]` is the other direction.
    let mut reads_of: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let mut read_by: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for t in &m.tables {
        for f in &t.fields {
            let me = NodeId { table: t.id.clone(), field: f.name.clone() };
            let mut sources = reads(m, &t.id, f);
            sources.retain(|s| known.contains(s));
            sources.dedup();
            for s in &sources {
                read_by.entry(s.clone()).or_default().push(me.clone());
            }
            reads_of.insert(me, sources);
        }
    }

    // Kahn's algorithm. Among the nodes that are ready, the one declared first runs first.
    let position: HashMap<&NodeId, usize> = nodes.iter().enumerate().map(|(i, n)| (n, i)).collect();
    let mut waiting: HashMap<&NodeId, usize> = nodes.iter().map(|n| (n, reads_of.get(n).map_or(0, |r| r.iter().collect::<HashSet<_>>().len()))).collect();
    let mut ready: BTreeSet<(usize, &NodeId)> = nodes.iter().filter(|n| waiting[n] == 0).map(|n| (position[n], n)).collect();
    let mut order: Vec<NodeId> = Vec::with_capacity(nodes.len());
    while let Some(&(p, n)) = ready.iter().next() {
        ready.remove(&(p, n));
        order.push(n.clone());
        let mut seen = HashSet::new();
        for next in read_by.get(n).into_iter().flatten().filter(|x| seen.insert(*x)) {
            if let Some(w) = waiting.get_mut(next) {
                *w -= 1;
                if *w == 0 {
                    ready.insert((position[next], next));
                }
            }
        }
    }
    if order.len() == nodes.len() {
        return Ok(Graph { order, read_by });
    }

    // Something is left over, so there is a cycle. Walk what each node reads until a node repeats.
    let done: HashSet<&NodeId> = order.iter().collect();
    for start in nodes.iter().filter(|n| !done.contains(n)) {
        let mut stack: Vec<&NodeId> = vec![start];
        let mut seen: HashSet<&NodeId> = HashSet::from([start]);
        while let Some(next) = reads_of.get(*stack.last().expect("the stack holds the start")).into_iter().flatten().find(|n| !done.contains(n)) {
            if let Some(at) = stack.iter().position(|n| *n == next) {
                let path: Vec<NodeId> = stack[at..].iter().map(|n| (*n).clone()).collect();
                let text = sentence(m, &path);
                return Err(CycleError { path, text });
            }
            if !seen.insert(next) {
                break;
            }
            stack.push(next);
        }
    }
    // A leftover node always reads another leftover node, so the walk above finds a cycle. If it ever
    // did not, the graph is still usable: the leftovers run last, in the order declared.
    let leftover: Vec<NodeId> = nodes.iter().filter(|n| !done.contains(n)).cloned().collect();
    let mut order = order.clone();
    order.extend(leftover);
    Ok(Graph { order, read_by })
}

impl Graph {
    /// The fields of a table in the order they must be computed: each after everything it reads.
    pub fn order(&self, table: &str) -> Vec<String> {
        self.order.iter().filter(|n| n.table == table).map(|n| n.field.clone()).collect()
    }

    /// Everything downstream of `changed`, in the order it must run. The changed nodes themselves are
    /// not in the answer.
    pub fn affected(&self, changed: &[NodeId]) -> Vec<NodeId> {
        let mut reached: HashSet<&NodeId> = HashSet::new();
        let mut queue: VecDeque<&NodeId> = changed.iter().collect();
        while let Some(n) = queue.pop_front() {
            for next in self.read_by.get(n).into_iter().flatten() {
                if reached.insert(next) {
                    queue.push_back(next);
                }
            }
        }
        self.order.iter().filter(|n| reached.contains(n) && !changed.contains(n)).cloned().collect()
    }
}
