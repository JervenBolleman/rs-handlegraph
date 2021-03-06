use crate::{
    handle::{Direction, Handle, NodeId},
    packed::*,
};

use crate::packed;

use super::{
    edges::EdgeListIx,
    graph::NARROW_PAGE_WIDTH,
    index::{NodeRecordId, OneBasedIndex, RecordIndex},
    occurrences::OccurListIx,
    sequence::{SeqRecordIx, Sequences},
};

/// The index into the underlying packed vector that is used to
/// represent the graph records that hold pointers to the two edge
/// lists for each node.
///
/// Each graph record takes up two elements, so a `GraphVecIx` is
/// always even.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct GraphVecIx(usize);

impl RecordIndex for GraphVecIx {
    const RECORD_WIDTH: usize = 2;

    #[inline]
    fn from_one_based_ix<I: OneBasedIndex>(ix: I) -> Option<Self> {
        ix.to_record_start(Self::RECORD_WIDTH).map(GraphVecIx)
    }

    #[inline]
    fn to_one_based_ix<I: OneBasedIndex>(self) -> I {
        I::from_record_start(self.0, Self::RECORD_WIDTH)
    }

    #[inline]
    fn record_ix(self, offset: usize) -> usize {
        self.0 + offset
    }
}

impl GraphVecIx {
    #[inline]
    pub(super) fn left_edges_ix(&self) -> usize {
        self.0
    }

    #[inline]
    pub(super) fn right_edges_ix(&self) -> usize {
        self.0 + 1
    }
}

#[derive(Debug, Clone)]
pub struct NodeIdIndexMap {
    deque: PackedDeque,
    max_id: u64,
    min_id: u64,
}

crate::impl_space_usage!(NodeIdIndexMap, [deque]);

impl Default for NodeIdIndexMap {
    fn default() -> Self {
        Self {
            deque: Default::default(),
            max_id: 0,
            min_id: std::u64::MAX,
        }
    }
}

impl NodeIdIndexMap {
    pub(super) fn iter(&self) -> packed::deque::Iter<'_> {
        self.deque.iter()
    }

    pub(super) fn len(&self) -> usize {
        self.deque.len()
    }

    fn clear_node_id(&mut self, id: NodeId) {
        let ix = u64::from(id) - self.min_id;
        self.deque.set(ix as usize, 0);
    }

    /// Appends the provided NodeId to the Node id -> Graph index map,
    /// with the given target `GraphRecordIx`.
    ///
    /// Returns `true` if the NodeId was successfully appended.
    pub fn append_node_id(
        &mut self,
        id: NodeId,
        next_ix: NodeRecordId,
    ) -> bool {
        let id = u64::from(id);
        if id == 0 {
            return false;
        }

        if self.deque.is_empty() {
            self.deque.push_back(0);
        } else {
            if id < self.min_id {
                let to_prepend = self.min_id - id;
                for _ in 0..to_prepend {
                    self.deque.push_front(0);
                }
            }

            if id > self.max_id {
                let ix = (id - self.min_id) as usize;
                if let Some(to_append) = ix.checked_sub(self.deque.len()) {
                    for _ in 0..=to_append {
                        self.deque.push_back(0);
                    }
                }
            }
        }

        self.min_id = self.min_id.min(id);
        self.max_id = self.max_id.max(id);

        let index = id - self.min_id;
        let value = next_ix;

        self.deque.set(index as usize, value.pack());

        true
    }

    #[inline]
    fn has_node<I: Into<NodeId>>(&self, id: I) -> bool {
        self.get_index(id).is_some()
    }

    #[inline]
    pub fn get_index<I: Into<NodeId>>(&self, id: I) -> Option<NodeRecordId> {
        let id = u64::from(id.into());
        if id < self.min_id || id > self.max_id {
            return None;
        }
        let index = id - self.min_id;
        let rec_id: NodeRecordId = self.deque.get_unpack(index as usize);

        if rec_id.is_null() {
            return None;
        }

        Some(rec_id)
    }
}

#[derive(Debug, Clone)]
pub struct NodeRecords {
    records_vec: PagedIntVec,
    id_index_map: NodeIdIndexMap,
    sequences: Sequences,
    removed_nodes: Vec<NodeId>,
    pub(super) node_occurrence_map: PagedIntVec,
}

crate::impl_space_usage!(
    NodeRecords,
    [
        records_vec,
        id_index_map,
        sequences,
        removed_nodes,
        node_occurrence_map
    ]
);

impl Default for NodeRecords {
    fn default() -> NodeRecords {
        Self {
            records_vec: PagedIntVec::new(NARROW_PAGE_WIDTH),
            id_index_map: Default::default(),
            sequences: Default::default(),
            removed_nodes: Vec::new(),
            node_occurrence_map: PagedIntVec::new(
                super::graph::NARROW_PAGE_WIDTH,
            ),
        }
    }
}

impl NodeRecords {
    #[inline]
    pub fn min_id(&self) -> u64 {
        self.id_index_map.min_id
    }

    #[inline]
    pub fn max_id(&self) -> u64 {
        self.id_index_map.max_id
    }

    pub fn nodes_iter(&self) -> packed::deque::Iter<'_> {
        self.id_index_map.iter()
    }

    #[inline]
    pub fn has_node<I: Into<NodeId>>(&self, id: I) -> bool {
        self.id_index_map.has_node(id)
    }

    #[inline]
    pub fn node_count(&self) -> usize {
        self.id_index_map.len()
    }

    #[inline]
    pub fn total_length(&self) -> usize {
        self.sequences.total_length()
    }

    /// Return the `GraphRecordIx` that will be used by the next node
    /// that's inserted into the graph.
    fn next_graph_ix(&self) -> NodeRecordId {
        let rec_count = self.records_vec.len();
        let rec_id = NodeRecordId::from_record_start(rec_count, 2);
        rec_id
    }

    pub(super) fn sequences(&self) -> &Sequences {
        &self.sequences
    }

    pub(super) fn sequences_mut(&mut self) -> &mut Sequences {
        &mut self.sequences
    }

    /// Append a new node graph record, using the provided
    /// `NodeRecordId` no ensure that the record index is correctly
    /// synced.
    #[must_use]
    fn append_node_graph_record(
        &mut self,
        g_rec_ix: NodeRecordId,
    ) -> Option<NodeRecordId> {
        if self.next_graph_ix() != g_rec_ix {
            return None;
        }
        self.records_vec.append(0);
        self.records_vec.append(0);
        self.node_occurrence_map.append(0);
        Some(g_rec_ix)
    }

    fn insert_node(&mut self, n_id: NodeId) -> Option<NodeRecordId> {
        if n_id == NodeId::from(0) {
            return None;
        }

        let next_ix = self.next_graph_ix();

        // Make sure the node ID is valid and doesn't already exist
        if !self.id_index_map.append_node_id(n_id, next_ix) {
            return None;
        }

        // append the sequence and graph records
        self.sequences.append_empty_record();
        let record_ix = self.append_node_graph_record(next_ix)?;

        Some(record_ix)
    }

    pub(super) fn clear_node_record(&mut self, n_id: NodeId) -> Option<()> {
        let rec_id = self.id_index_map.get_index(n_id)?;

        let occ_map_ix = rec_id.to_record_ix(1, 0)?;
        let rec_ix = rec_id.to_record_ix(2, 0)?;
        let seq_ix = SeqRecordIx::from_one_based_ix(rec_id)?;

        // clear node occurrence heads
        self.node_occurrence_map.set(occ_map_ix, 0);

        // clear node record/edge list heads
        self.records_vec.set(rec_ix, 0);
        self.records_vec.set(rec_ix, 1);

        // clear sequence record
        self.sequences.clear_record(seq_ix);

        self.id_index_map.clear_node_id(n_id);

        self.removed_nodes.push(n_id);

        Some(())
    }

    #[inline]
    pub(super) fn get_edge_list(
        &self,
        rec_id: NodeRecordId,
        dir: Direction,
    ) -> EdgeListIx {
        match GraphVecIx::from_one_based_ix(rec_id) {
            None => EdgeListIx::null(),
            Some(vec_ix) => {
                let ix = match dir {
                    Direction::Right => vec_ix.right_edges_ix(),
                    Direction::Left => vec_ix.left_edges_ix(),
                };

                self.records_vec.get_unpack(ix)
            }
        }
    }

    #[inline]
    pub(super) fn set_edge_list(
        &mut self,
        rec_id: NodeRecordId,
        dir: Direction,
        new_edge: EdgeListIx,
    ) -> Option<()> {
        let vec_ix = GraphVecIx::from_one_based_ix(rec_id)?;

        let ix = match dir {
            Direction::Right => vec_ix.right_edges_ix(),
            Direction::Left => vec_ix.left_edges_ix(),
        };

        self.records_vec.set_pack(ix, new_edge);
        Some(())
    }

    #[inline]
    pub(super) fn get_node_edge_lists(
        &self,
        rec_id: NodeRecordId,
    ) -> Option<(EdgeListIx, EdgeListIx)> {
        let vec_ix = GraphVecIx::from_one_based_ix(rec_id)?;

        let left = vec_ix.left_edges_ix();
        let left = self.records_vec.get_unpack(left);

        let right = vec_ix.right_edges_ix();
        let right = self.records_vec.get_unpack(right);

        Some((left, right))
    }

    #[allow(dead_code)]
    pub(super) fn set_node_edge_lists(
        &mut self,
        rec_id: NodeRecordId,
        left: EdgeListIx,
        right: EdgeListIx,
    ) -> Option<()> {
        let vec_ix = GraphVecIx::from_one_based_ix(rec_id)?;

        let left_ix = vec_ix.left_edges_ix();
        let right_ix = vec_ix.right_edges_ix();
        self.records_vec.set_pack(left_ix, left);
        self.records_vec.set_pack(right_ix, right);

        Some(())
    }

    #[inline]
    pub(super) fn update_node_edge_lists<F>(
        &mut self,
        rec_id: NodeRecordId,
        f: F,
    ) -> Option<()>
    where
        F: Fn(EdgeListIx, EdgeListIx) -> (EdgeListIx, EdgeListIx),
    {
        let vec_ix = GraphVecIx::from_one_based_ix(rec_id)?;

        let (left_rec, right_rec) = self.get_node_edge_lists(rec_id)?;

        let (new_left, new_right) = f(left_rec, right_rec);

        let left_ix = vec_ix.left_edges_ix();
        let right_ix = vec_ix.right_edges_ix();
        self.records_vec.set_pack(left_ix, new_left);
        self.records_vec.set_pack(right_ix, new_right);

        Some(())
    }

    pub(super) fn create_node<I: Into<NodeId>>(
        &mut self,
        n_id: I,
        seq: &[u8],
    ) -> Option<NodeRecordId> {
        let n_id = n_id.into();
        // update the node ID/graph index map
        let g_ix = self.insert_node(n_id)?;

        // insert the sequence
        self.sequences.add_sequence(g_ix, seq);

        Some(g_ix)
    }

    pub(super) fn append_empty_node(&mut self) -> NodeId {
        let n_id = NodeId::from(self.id_index_map.max_id + 1);
        let _g_ix = self.insert_node(n_id).unwrap();
        n_id
    }

    #[inline]
    pub(crate) fn handle_record(&self, h: Handle) -> Option<NodeRecordId> {
        self.id_index_map.get_index(h.id())
    }

    #[inline]
    pub(crate) fn node_record_occur(
        &self,
        rec_id: NodeRecordId,
    ) -> Option<OccurListIx> {
        let vec_ix = rec_id.to_zero_based()?;
        Some(self.node_occurrence_map.get_unpack(vec_ix))
    }

    /// Maps a handle into its corresponding occurrence record
    /// pointer, if the node for the handle exists in the PackedGraph.
    #[inline]
    pub(crate) fn handle_occur_record(&self, h: Handle) -> Option<OccurListIx> {
        self.handle_record(h)
            .and_then(|r| self.node_record_occur(r))
    }
}
