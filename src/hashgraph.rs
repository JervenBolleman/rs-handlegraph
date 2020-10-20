use bstr::BString;
use std::collections::HashMap;

use gfa::{
    gfa::{Link, Segment, GFA},
    optfields::OptFields,
};

use crate::{
    handle::{Direction, Edge, Handle, NodeId},
    handlegraph::HandleGraph,
    mutablehandlegraph::MutableHandleGraph,
    pathgraph::PathHandleGraph,
};

use bio::alphabets::dna;

pub type PathId = i64;

#[derive(Debug, Clone, PartialEq)]
pub enum PathStep {
    Front(i64),
    End(i64),
    Step(i64, usize),
}

impl PathStep {
    pub fn index(&self) -> Option<usize> {
        if let Self::Step(_, ix) = self {
            Some(*ix)
        } else {
            None
        }
    }

    pub fn path_id(&self) -> PathId {
        match self {
            Self::Front(i) => *i,
            Self::End(i) => *i,
            Self::Step(i, _) => *i,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub sequence: BString,
    pub left_edges: Vec<Handle>,
    pub right_edges: Vec<Handle>,
    pub occurrences: HashMap<PathId, usize>,
}

impl Node {
    pub fn new(sequence: &[u8]) -> Node {
        Node {
            sequence: sequence.into(),
            left_edges: vec![],
            right_edges: vec![],
            occurrences: HashMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct Path {
    pub path_id: PathId,
    pub name: BString,
    pub is_circular: bool,
    pub nodes: Vec<Handle>,
}

impl Path {
    fn new<T: Into<BString>>(
        name: T,
        path_id: PathId,
        is_circular: bool,
    ) -> Self {
        Path {
            name: name.into(),
            path_id,
            is_circular,
            nodes: vec![],
        }
    }

    fn lookup_step_handle(&self, step: &PathStep) -> Option<Handle> {
        match step {
            PathStep::Front(_) => None,
            PathStep::End(_) => None,
            PathStep::Step(_, ix) => Some(self.nodes[*ix]),
        }
    }
}

#[derive(Default, Debug)]
pub struct HashGraph {
    pub max_id: NodeId,
    pub min_id: NodeId,
    pub graph: HashMap<NodeId, Node>,
    pub path_id: HashMap<Vec<u8>, i64>,
    pub paths: HashMap<i64, Path>,
}

impl HashGraph {
    pub fn new() -> HashGraph {
        HashGraph {
            max_id: NodeId::from(0),
            min_id: NodeId::from(std::u64::MAX),
            ..Default::default()
        }
    }

    fn add_gfa_segment<'a, 'b, T: OptFields>(
        &'a mut self,
        seg: &'b Segment<usize, T>,
    ) {
        self.create_handle(&seg.sequence, seg.name as u64);
    }

    fn add_gfa_link<T: OptFields>(&mut self, link: &Link<usize, T>) {
        let left = Handle::new(link.from_segment as u64, link.from_orient);
        let right = Handle::new(link.to_segment as u64, link.to_orient);

        self.create_edge(&Edge(left, right));
    }

    fn add_gfa_path<T: OptFields>(&mut self, path: &gfa::gfa::Path<usize, T>) {
        let path_id = self.create_path_handle(&path.path_name, false);
        for (name, orient) in path.iter() {
            self.append_step(&path_id, Handle::new(name as u64, orient));
        }
    }

    pub fn from_gfa<T: OptFields>(gfa: &GFA<usize, T>) -> HashGraph {
        let mut graph = Self::new();
        gfa.segments.iter().for_each(|s| graph.add_gfa_segment(s));
        gfa.links.iter().for_each(|l| graph.add_gfa_link(l));
        gfa.paths.iter().for_each(|p| graph.add_gfa_path(p));
        graph
    }

    pub fn print_path(&self, path_id: &PathId) {
        let path = self.paths.get(&path_id).unwrap();
        println!("Path\t{}", path_id);
        for (ix, handle) in path.nodes.iter().enumerate() {
            let node = self.get_node(&handle.id()).unwrap();
            if ix != 0 {
                print!(" -> ");
            }
            print!("{}", node.sequence);
        }

        println!();
    }

    pub fn print_occurrences(&self) {
        self.handles_iter().for_each(|h| {
            let node = self.get_node(&h.id()).unwrap();
            println!("{} - {:?}", node.sequence, node.occurrences);
        });
    }

    pub fn get_node(&self, node_id: &NodeId) -> Option<&Node> {
        self.graph.get(node_id)
    }

    pub fn get_node_unchecked(&self, node_id: &NodeId) -> &Node {
        self.graph.get(node_id).unwrap_or_else(|| {
            panic!("Tried getting a node that doesn't exist, ID: {:?}", node_id)
        })
    }

    pub fn get_node_mut(&mut self, node_id: &NodeId) -> Option<&mut Node> {
        self.graph.get_mut(node_id)
    }
}

impl HandleGraph for HashGraph {
    fn has_node(&self, node_id: NodeId) -> bool {
        self.graph.contains_key(&node_id)
    }

    fn sequence(&self, handle: Handle) -> Vec<u8> {
        let seq: &[u8] =
            &self.get_node_unchecked(&handle.id()).sequence.as_ref();
        if handle.is_reverse() {
            dna::revcomp(seq)
        } else {
            seq.into()
        }
    }

    fn sequence_slice(&self, handle: Handle) -> &[u8] {
        &self.get_node_unchecked(&handle.id()).sequence.as_ref()
    }

    fn length(&self, handle: Handle) -> usize {
        self.sequence(handle).len()
    }

    fn degree(&self, handle: Handle, dir: Direction) -> usize {
        let n = self.get_node_unchecked(&handle.id());
        match dir {
            Direction::Right => n.right_edges.len(),
            Direction::Left => n.left_edges.len(),
        }
    }

    fn node_count(&self) -> usize {
        self.graph.len()
    }

    fn min_node_id(&self) -> NodeId {
        self.min_id
    }

    fn max_node_id(&self) -> NodeId {
        self.max_id
    }

    fn edge_count(&self) -> usize {
        self.graph
            .iter()
            .fold(0, |a, (_, v)| a + v.left_edges.len() + v.right_edges.len())
    }

    fn handle_edges_iter<'a>(
        &'a self,
        handle: Handle,
        dir: Direction,
    ) -> Box<dyn Iterator<Item = Handle> + 'a> {
        let node = self.get_node_unchecked(&handle.id());

        let handles = match (dir, handle.is_reverse()) {
            (Direction::Left, true) => &node.right_edges,
            (Direction::Left, false) => &node.left_edges,
            (Direction::Right, true) => &node.left_edges,
            (Direction::Right, false) => &node.right_edges,
        };

        Box::new(handles.iter().map(move |h| {
            if dir == Direction::Left {
                h.flip()
            } else {
                *h
            }
        }))
    }

    fn handles_iter<'a>(&'a self) -> Box<dyn Iterator<Item = Handle> + 'a> {
        Box::new(self.graph.keys().map(|i| Handle::pack(*i, false)))
    }

    fn edges_iter<'a>(&'a self) -> Box<dyn Iterator<Item = Edge> + 'a> {
        use Direction::*;

        let handles = self.handles_iter();

        let neighbors = move |handle: Handle| {
            let right_neighbors = self
                .handle_edges_iter(handle, Right)
                .filter_map(move |next| {
                    if handle.id() <= next.id() {
                        Some(Edge::edge_handle(handle, next))
                    } else {
                        None
                    }
                });

            let left_neighbors = self
                .handle_edges_iter(handle, Left)
                .filter_map(move |prev| {
                    if (handle.id() < prev.id())
                        || (handle.id() == prev.id() && prev.is_reverse())
                    {
                        Some(Edge::edge_handle(prev, handle))
                    } else {
                        None
                    }
                });

            right_neighbors.chain(left_neighbors)
        };

        Box::new(handles.map(neighbors).flatten())
    }
}

impl MutableHandleGraph for HashGraph {
    fn append_handle(&mut self, sequence: &[u8]) -> Handle {
        self.create_handle(sequence, self.max_id + 1)
    }

    fn create_handle<T: Into<NodeId>>(
        &mut self,
        seq: &[u8],
        node_id: T,
    ) -> Handle {
        let id: NodeId = node_id.into();

        if seq.is_empty() {
            panic!("Tried to add empty handle");
        }
        self.graph.insert(id, Node::new(seq));
        self.max_id = std::cmp::max(self.max_id, id);
        self.min_id = std::cmp::min(self.min_id, id);
        Handle::pack(id, false)
    }

    fn create_edge(&mut self, Edge(left, right): &Edge) {
        let add_edge = {
            let left_node = self
                .graph
                .get(&left.id())
                .expect("Node doesn't exist for the given handle");

            None == left_node.right_edges.iter().find(|h| *h == right)
        };

        if add_edge {
            let left_node = self
                .graph
                .get_mut(&left.id())
                .expect("Node doesn't exist for the given handle");
            if left.is_reverse() {
                left_node.left_edges.push(*right);
            } else {
                left_node.right_edges.push(*right);
            }
            if left != &right.flip() {
                let right_node = self
                    .graph
                    .get_mut(&right.id())
                    .expect("Node doesn't exist for the given handle");
                if right.is_reverse() {
                    right_node.right_edges.push(left.flip());
                } else {
                    right_node.left_edges.push(left.flip());
                }
            }
        }
    }

    fn divide_handle(
        &mut self,
        handle: Handle,
        mut offsets: Vec<usize>,
    ) -> Vec<Handle> {
        let mut result = vec![handle];
        let node_len = self.length(handle);
        let sequence = self.sequence(handle);

        let fwd_handle = handle.forward();

        // Push the node length as a last offset to make constructing
        // the ranges nicer
        offsets.push(node_len);

        let fwd_offsets: Vec<usize> = if handle.is_reverse() {
            offsets.iter().map(|o| node_len - o).collect()
        } else {
            offsets
        };

        // staggered zip of the offsets with themselves to make the ranges
        let ranges: Vec<_> = fwd_offsets
            .iter()
            .zip(fwd_offsets.iter().skip(1))
            .map(|(&p, &n)| p..n)
            .collect();

        // TODO it should be possible to do this without creating new
        // strings and collecting into a vec

        let subseqs: Vec<BString> =
            ranges.into_iter().map(|r| sequence[r].into()).collect();

        for seq in subseqs {
            let h = self.append_handle(&seq);
            result.push(h);
        }

        // move the outgoing edges to the last new segment
        // empty the existing right edges of the original node
        let mut orig_rights = std::mem::take(
            &mut self.get_node_mut(&handle.id()).unwrap().right_edges,
        );

        let new_rights = &mut self
            .get_node_mut(&result.last().unwrap().id())
            .unwrap()
            .right_edges;
        // and swap with the new right edges
        std::mem::swap(&mut orig_rights, new_rights);

        // shrink the sequence of the starting handle
        let orig_node = &mut self.get_node_mut(&handle.id()).unwrap();
        orig_node.sequence = orig_node.sequence[0..fwd_offsets[0]].into();

        // update backwards references
        // first collect all the handles whose nodes we need to update
        let last_neighbors: Vec<_> = self
            .handle_edges_iter(*result.last().unwrap(), Direction::Right)
            .collect();

        // And perform the update
        for h in last_neighbors {
            let node = &mut self.get_node_mut(&h.id()).unwrap();
            let neighbors = if h.is_reverse() {
                &mut node.right_edges
            } else {
                &mut node.left_edges
            };

            for bwd in neighbors.iter_mut() {
                if *bwd == fwd_handle.flip() {
                    *bwd = result.last().unwrap().flip();
                }
            }
        }

        // create edges between the new segments
        for (this, next) in result.iter().zip(result.iter().skip(1)) {
            self.create_edge(&Edge(*this, *next));
        }

        // update paths and path occurrences

        // TODO this is probably not
        // correct, and it's silly to clone the results all the time
        let affected_paths: Vec<(i64, usize)> = self
            .get_node_unchecked(&handle.id())
            .occurrences
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();

        for (path_id, ix) in affected_paths.into_iter() {
            let step = PathStep::Step(path_id, ix);
            self.rewrite_segment(&step, &step, result.clone());
        }

        result
    }
}

impl HashGraph {
    pub fn get_path(&self, path_id: &PathId) -> Option<&Path> {
        self.paths.get(path_id)
    }

    pub fn get_path_unchecked(&self, path_id: &PathId) -> &Path {
        self.paths
            .get(path_id)
            .unwrap_or_else(|| panic!("Tried to look up nonexistent path:"))
    }
}

impl PathHandleGraph for HashGraph {
    type PathHandle = PathId;
    type StepHandle = PathStep;

    fn path_count(&self) -> usize {
        self.path_id.len()
    }

    fn has_path(&self, name: &[u8]) -> bool {
        self.path_id.contains_key(name)
    }

    fn name_to_path_handle(&self, name: &[u8]) -> Option<Self::PathHandle> {
        self.path_id.get(name).copied()
    }

    fn path_handle_to_name(&self, path_id: &Self::PathHandle) -> &[u8] {
        self.get_path_unchecked(path_id).name.as_slice()
    }

    fn is_circular(&self, path_id: &Self::PathHandle) -> bool {
        self.get_path_unchecked(path_id).is_circular
    }

    fn step_count(&self, path_id: &Self::PathHandle) -> usize {
        self.get_path_unchecked(path_id).nodes.len()
    }

    fn handle_of_step(&self, step: &Self::StepHandle) -> Option<Handle> {
        self.get_path_unchecked(&step.path_id())
            .lookup_step_handle(step)
    }

    fn path_handle_of_step(&self, step: &Self::StepHandle) -> Self::PathHandle {
        step.path_id()
    }

    fn path_begin(&self, path: &Self::PathHandle) -> Self::StepHandle {
        PathStep::Step(*path, 0)
    }

    fn path_end(&self, path: &Self::PathHandle) -> Self::StepHandle {
        PathStep::End(*path)
    }

    fn path_back(&self, path: &Self::PathHandle) -> Self::StepHandle {
        PathStep::Step(*path, self.step_count(path) - 1)
    }

    fn path_front_end(&self, path: &Self::PathHandle) -> Self::StepHandle {
        PathStep::Front(*path)
    }

    fn has_next_step(&self, step: &Self::StepHandle) -> bool {
        matches!(step, PathStep::End(_))
    }

    fn has_previous_step(&self, step: &Self::StepHandle) -> bool {
        matches!(step, PathStep::Front(_))
    }

    fn next_step(&self, step: &Self::StepHandle) -> Self::StepHandle {
        match step {
            PathStep::Front(pid) => self.path_begin(pid),
            PathStep::End(pid) => self.path_end(pid),
            PathStep::Step(pid, ix) => {
                if *ix < self.step_count(pid) - 1 {
                    PathStep::Step(*pid, ix + 1)
                } else {
                    self.path_end(pid)
                }
            }
        }
    }

    fn previous_step(&self, step: &Self::StepHandle) -> Self::StepHandle {
        match step {
            PathStep::Front(pid) => self.path_front_end(pid),
            PathStep::End(pid) => self.path_back(pid),
            PathStep::Step(pid, ix) => {
                if *ix > 0 {
                    PathStep::Step(*pid, ix - 1)
                } else {
                    self.path_end(pid)
                }
            }
        }
    }

    fn destroy_path(&mut self, path: &Self::PathHandle) {
        let p: &Path = self.paths.get(&path).unwrap();

        for handle in p.nodes.iter() {
            let node: &mut Node = self.graph.get_mut(&handle.id()).unwrap();
            node.occurrences.remove(path);
        }
        self.paths.remove(&path);
    }

    fn create_path_handle(
        &mut self,
        name: &[u8],
        is_circular: bool,
    ) -> Self::PathHandle {
        let path_id = self.paths.len() as i64;
        let path = Path::new(name, path_id, is_circular);
        self.path_id.insert(name.into(), path_id);
        self.paths.insert(path_id, path);
        path_id
    }

    fn append_step(
        &mut self,
        path_id: &Self::PathHandle,
        to_append: Handle,
    ) -> Self::StepHandle {
        let path: &mut Path = self.paths.get_mut(path_id).unwrap();
        path.nodes.push(to_append);
        let step = (*path_id, path.nodes.len() - 1);
        let node: &mut Node = self.graph.get_mut(&to_append.id()).unwrap();
        node.occurrences.insert(step.0, step.1);
        PathStep::Step(*path_id, path.nodes.len() - 1)
    }

    fn prepend_step(
        &mut self,
        path_id: &Self::PathHandle,
        to_prepend: Handle,
    ) -> Self::StepHandle {
        let path: &mut Path = self.paths.get_mut(path_id).unwrap();
        // update occurrences in nodes already in the graph
        for h in path.nodes.iter() {
            let node: &mut Node = self.graph.get_mut(&h.id()).unwrap();
            *node.occurrences.get_mut(path_id).unwrap() += 1;
        }
        path.nodes.insert(0, to_prepend);
        let node: &mut Node = self.graph.get_mut(&to_prepend.id()).unwrap();
        node.occurrences.insert(*path_id, 0);
        PathStep::Step(*path_id, 0)
    }

    fn rewrite_segment(
        &mut self,
        begin: &Self::StepHandle,
        end: &Self::StepHandle,
        new_segment: Vec<Handle>,
    ) -> (Self::StepHandle, Self::StepHandle) {
        // extract the index range from the begin and end handles

        if begin.path_id() != end.path_id() {
            panic!("Tried to rewrite path segment between two different paths");
        }

        let path_id = begin.path_id();
        let path_len = self.paths.get(&path_id).unwrap().nodes.len();

        let step_index = |s: &Self::StepHandle| match s {
            PathStep::Front(_) => 0,
            PathStep::End(_) => path_len - 1,
            PathStep::Step(_, i) => *i,
        };

        let l = step_index(begin);
        let r = step_index(end);

        let range = l..=r;

        // first delete the occurrences of the nodes in the range
        for handle in self
            .paths
            .get(&path_id)
            .unwrap()
            .nodes
            .iter()
            .skip(l)
            .take(r - l + 1)
        {
            let node: &mut Node = self.graph.get_mut(&handle.id()).unwrap();
            node.occurrences.remove(&path_id);
        }

        // get a &mut to the path's vector of handles
        let handles: &mut Vec<Handle> =
            &mut self.paths.get_mut(&path_id).unwrap().nodes;

        let r = l + new_segment.len();
        // replace the range of the path's handle vector with the new segment
        handles.splice(range, new_segment);

        // update occurrences
        for (ix, handle) in
            self.paths.get(&path_id).unwrap().nodes.iter().enumerate()
        {
            let node: &mut Node = self.graph.get_mut(&handle.id()).unwrap();
            node.occurrences.insert(path_id, ix);
        }

        // return the new beginning and end step handles: even if the
        // input steps were Front and/or End, the output steps exist
        // on the path
        (PathStep::Step(path_id, l), PathStep::Step(path_id, r))
    }

    fn paths_iter<'a>(
        &'a self,
    ) -> Box<dyn Iterator<Item = &'a Self::PathHandle> + 'a> {
        Box::new(self.paths.keys())
    }

    fn occurrences_iter<'a>(
        &'a self,
        handle: Handle,
    ) -> Box<dyn Iterator<Item = Self::StepHandle> + 'a> {
        let node: &Node = self.get_node_unchecked(&handle.id());
        Box::new(node.occurrences.iter().map(|(k, v)| PathStep::Step(*k, *v)))
    }

    fn steps_iter<'a>(
        &'a self,
        path_handle: &'a Self::PathHandle,
    ) -> Box<dyn Iterator<Item = Self::StepHandle> + 'a> {
        let path = self.get_path_unchecked(path_handle);
        Box::new(
            path.nodes
                .iter()
                .enumerate()
                .map(move |(i, _)| PathStep::Step(*path_handle, i)),
        )
    }
}
