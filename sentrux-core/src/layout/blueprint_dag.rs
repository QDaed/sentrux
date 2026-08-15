//! DAG ordering for top-level folders in blueprint layout.
//!
//! Extracted from blueprint.rs to keep each file under 500 lines.
//! Computes a topological layer order from the import/call graph so that
//! upstream modules appear left/top of downstream modules.

use crate::core::snapshot::Snapshot;
use crate::core::types::FileNode;
use std::collections::{HashMap, HashSet, VecDeque};

/// Map every file in the tree to its top-level folder name.
/// Files directly under root map to `"__root__"`.
fn build_file_to_folder_map<'a>(
    children: &'a [FileNode],
    dirs: &mut Vec<&'a FileNode>,
) -> HashMap<String, String> {
    let mut file_to_folder: HashMap<String, String> = HashMap::new();
    for c in children {
        if c.is_dir {
            dirs.push(c);
            walk_files(c, &c.path, &mut file_to_folder);
        } else {
            file_to_folder.insert(c.path.clone(), "__root__".to_string());
        }
    }
    file_to_folder
}

fn walk_files(n: &FileNode, folder: &str, map: &mut HashMap<String, String>) {
    if !n.is_dir {
        map.insert(n.path.clone(), folder.to_string());
        return;
    }
    if let Some(children) = &n.children {
        for c in children {
            walk_files(c, folder, map);
        }
    }
}

/// Build folder-level adjacency from import and call graphs.
fn build_folder_adjacency(
    dirs: &[&FileNode],
    file_to_folder: &HashMap<String, String>,
    snapshot: &Snapshot,
) -> HashMap<String, HashSet<String>> {
    let mut out_edges: HashMap<String, HashSet<String>> = HashMap::new();
    for d in dirs {
        out_edges.insert(d.path.clone(), HashSet::new());
    }

    for e in &snapshot.import_graph {
        add_folder_edge(&mut out_edges, file_to_folder, &e.from_file, &e.to_file);
    }
    for e in &snapshot.call_graph {
        add_folder_edge(&mut out_edges, file_to_folder, &e.from_file, &e.to_file);
    }
    out_edges
}

fn add_folder_edge(
    out_edges: &mut HashMap<String, HashSet<String>>,
    file_to_folder: &HashMap<String, String>,
    from_file: &str,
    to_file: &str,
) {
    if let (Some(ff), Some(tf)) = (file_to_folder.get(from_file), file_to_folder.get(to_file)) {
        if ff != tf && out_edges.contains_key(ff.as_str()) && out_edges.contains_key(tf.as_str()) {
            out_edges.get_mut(ff.as_str()).unwrap().insert(tf.clone());
        }
    }
}

/// DFS-based cycle breaking. Returns a DAG (back-edges removed).
fn break_cycles(
    dirs: &[&FileNode],
    out_edges: &HashMap<String, HashSet<String>>,
) -> HashMap<String, Vec<String>> {
    const WHITE: u8 = 0;

    let mut color: HashMap<String, u8> = HashMap::new();
    let mut dag_out: HashMap<String, Vec<String>> = HashMap::new();
    for d in dirs {
        color.insert(d.path.clone(), WHITE);
        dag_out.insert(d.path.clone(), Vec::new());
    }

    fn dfs(
        u: &str,
        out_edges: &HashMap<String, HashSet<String>>,
        color: &mut HashMap<String, u8>,
        dag_out: &mut HashMap<String, Vec<String>>,
    ) {
        const WHITE: u8 = 0;
        const GRAY: u8 = 1;
        const BLACK: u8 = 2;
        // Use get_mut to avoid allocating a new String key on every color update.
        // The key is guaranteed to exist because we pre-populated color in break_cycles.
        if let Some(c) = color.get_mut(u) {
            *c = GRAY;
        }
        if let Some(edges) = out_edges.get(u) {
            let mut sorted: Vec<&String> = edges.iter().collect();
            sorted.sort();
            for v in sorted {
                let c = color.get(v.as_str()).copied().unwrap_or(WHITE);
                if c == WHITE {
                    dag_out.get_mut(u).unwrap().push(v.clone());
                    dfs(v, out_edges, color, dag_out);
                } else if c == BLACK {
                    dag_out.get_mut(u).unwrap().push(v.clone());
                }
                // GRAY -> back-edge, skip
            }
        }
        if let Some(c) = color.get_mut(u) {
            *c = BLACK;
        }
    }

    let dir_paths: Vec<String> = dirs.iter().map(|d| d.path.clone()).collect();
    for dp in &dir_paths {
        if color.get(dp.as_str()).copied().unwrap_or(WHITE) == WHITE {
            dfs(dp, out_edges, &mut color, &mut dag_out);
        }
    }
    dag_out
}

/// Kahn's algorithm toposort. Appends isolated nodes at the end.
fn toposort_kahn(dirs: &[&FileNode], dag_out: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut in_deg: HashMap<String, usize> = HashMap::new();
    for d in dirs {
        in_deg.insert(d.path.clone(), 0);
    }
    for targets in dag_out.values() {
        for t in targets {
            *in_deg.entry(t.clone()).or_insert(0) += 1;
        }
    }

    let mut zero_deg: Vec<String> = in_deg
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(n, _)| n.clone())
        .collect();
    zero_deg.sort();
    let mut queue: VecDeque<String> = zero_deg.into_iter().collect();
    let mut topo: Vec<String> = Vec::new();

    while let Some(u) = queue.pop_front() {
        topo.push(u.clone());
        if let Some(targets) = dag_out.get(&u) {
            let mut sorted_targets: Vec<&String> = targets.iter().collect();
            sorted_targets.sort();
            for v in sorted_targets {
                // Use if-let instead of unwrap to avoid panic if dag_out
                // somehow contains a target not in in_deg. [H13 fix]
                if let Some(nd) = in_deg.get_mut(v.as_str()) {
                    *nd -= 1;
                    if *nd == 0 {
                        queue.push_back(v.clone());
                    }
                }
            }
        }
    }
    // Append any not reached (isolated)
    let topo_set: HashSet<String> = topo.iter().cloned().collect();
    for d in dirs {
        if !topo_set.contains(&d.path) {
            topo.push(d.path.clone());
        }
    }
    topo
}

/// Longest-path layer assignment and final order computation.
fn assign_layers(
    dirs: &[&FileNode],
    dag_out: &HashMap<String, Vec<String>>,
    topo: &[String],
) -> HashMap<String, usize> {
    let mut layer: HashMap<String, usize> = HashMap::new();
    for d in dirs {
        layer.insert(d.path.clone(), 0);
    }
    for u in topo {
        let ul = layer.get(u).copied().unwrap_or(0);
        if let Some(targets) = dag_out.get(u) {
            for v in targets {
                let vl = layer.get(v).copied().unwrap_or(0);
                if vl < ul + 1 {
                    layer.insert(v.clone(), ul + 1);
                }
            }
        }
    }

    // Order = layer (primary), then topo index (secondary).
    let layer_stride = topo.len().max(1);
    let mut order = HashMap::new();
    for (i, path) in topo.iter().enumerate() {
        let l = layer.get(path).copied().unwrap_or(0);
        order.insert(path.clone(), l * layer_stride + i);
    }
    order
}

/// Compute DAG-based ordering for top-level folders.
///
/// Uses import + call graphs to determine which folders are upstream/downstream,
/// then assigns a numeric order so upstream folders sort first in the layout.
pub fn compute_dag_order(root: &FileNode, snapshot: &Snapshot) -> HashMap<String, usize> {
    let children = match &root.children {
        Some(c) => c,
        None => return HashMap::new(),
    };

    let mut dirs: Vec<&FileNode> = Vec::new();
    let file_to_folder = build_file_to_folder_map(children, &mut dirs);
    let out_edges = build_folder_adjacency(&dirs, &file_to_folder, snapshot);
    let dag_out = break_cycles(&dirs, &out_edges);
    let topo = toposort_kahn(&dirs, &dag_out);
    assign_layers(&dirs, &dag_out, &topo)
}
