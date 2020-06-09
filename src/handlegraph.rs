use crate::handle::{Direction, Edge, Handle, NodeId};

// kinda based on libbdsg's hashgraph

pub trait HandleGraph {
    fn has_node(&self, node_id: NodeId) -> bool;

    // fn get_id(&self, handle: &Handle) -> NodeId;
    // fn get_is_reverse(&self, handle: &Handle) -> bool;

    fn get_length(&self, handle: &Handle) -> usize;

    fn get_sequence(&self, handle: &Handle) -> &str;

    fn get_subsequence(
        &self,
        handle: &Handle,
        index: usize,
        size: usize,
    ) -> &str {
        &self.get_sequence(handle)[index..index + size]
    }

    fn get_base(&self, handle: &Handle, index: usize) -> char {
        char::from(self.get_sequence(handle).as_bytes()[index])
    }

    fn get_node_count(&self) -> usize;
    fn min_node_id(&self) -> NodeId;
    fn max_node_id(&self) -> NodeId;

    fn get_degree(&self, handle: &Handle, dir: Direction) -> usize {
        let mut count = 0;
        self.follow_edges(handle, dir, |_| {
            count += 1;
            true
        });

        count
    }

    fn has_edge(&self, left: &Handle, right: &Handle) -> bool {
        let mut found = false;

        self.follow_edges(left, Direction::Right, |h| {
            if h == right {
                found = true;
                return false;
            }
            true
        });

        found
    }

    fn get_edge_count(&self) -> usize;

    fn get_total_length(&self) -> usize;

    fn traverse_edge_handle(&self, edge: &Edge, left: &Handle) -> Handle;

    fn handle_edges_iter_impl<'a>(
        &'a self,
        handle: Handle,
        dir: Direction,
    ) -> Box<dyn FnMut() -> Option<Handle> + 'a>;

    fn handle_iter_impl<'a>(
        &'a self,
    ) -> Box<dyn FnMut() -> Option<Handle> + 'a>;

    fn follow_edges<F>(&self, handle: &Handle, dir: Direction, f: F) -> bool
    where
        F: FnMut(&Handle) -> bool;

    fn for_each_handle<F>(&self, f: F) -> bool
    where
        F: FnMut(&Handle) -> bool;

    fn for_each_edge<F>(&self, f: F) -> bool
    where
        F: FnMut(&Edge) -> bool;
}

pub fn handle_edges_iter<'a, T>(
    graph: &'a T,
    handle: Handle,
    dir: Direction,
) -> impl Iterator<Item = Handle> + 'a
where
    T: HandleGraph,
{
    std::iter::from_fn(graph.handle_edges_iter_impl(handle, dir))
}

pub fn handle_iter<'a, T>(
    graph: &'a T,
    handle: Handle,
    dir: Direction,
) -> impl Iterator<Item = Handle> + 'a
where
    T: HandleGraph,
{
    std::iter::from_fn(graph.handle_iter_impl())
}
