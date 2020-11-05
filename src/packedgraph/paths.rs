#![allow(dead_code)]
#![allow(unused_imports)]

use rayon::prelude::*;

use crate::{
    handle::{Direction, Edge, Handle, NodeId},
    handlegraph::*,
};

use fnv::FnvHashMap;

use super::{
    NodeRecordId, OneBasedIndex, PackedDoubleList, PackedList, PackedListIter,
    RecordIndex,
};

use super::NodeIdIndexMap;

use crate::pathhandlegraph::*;

use crate::packed;
use crate::packed::*;

mod packedpath;
mod properties;

pub use self::packedpath::*;
pub use self::properties::*;

pub use self::packedpath::StepUpdate;

#[derive(Debug, Clone)]
pub struct PackedPathNames {
    // TODO compress the names; don't store entire Vec<u8>s
    name_id_map: FnvHashMap<Vec<u8>, PathId>,
    names: PackedIntVec,
    lengths: PackedIntVec,
    offsets: PagedIntVec,
}

impl Default for PackedPathNames {
    fn default() -> Self {
        PackedPathNames {
            name_id_map: Default::default(),
            names: Default::default(),
            lengths: Default::default(),
            offsets: PagedIntVec::new(super::graph::NARROW_PAGE_WIDTH),
        }
    }
}

impl PackedPathNames {
    pub(super) fn add_name(&mut self, name: &[u8]) -> PathId {
        let path_id = PathId(self.lengths.len() as u64);

        self.name_id_map.insert(name.into(), path_id);

        let name_len = name.len() as u64;
        let name_offset = self.lengths.len() as u64;
        self.lengths.append(name_len);
        self.offsets.append(name_offset);

        name.iter().for_each(|&b| self.names.append(b as u64));

        path_id
    }

    pub(super) fn name_iter(
        &self,
        id: PathId,
    ) -> Option<packed::vector::IterView<'_, u8>> {
        let vec_ix = id.0 as usize;
        if vec_ix >= self.lengths.len() {
            return None;
        }

        let offset = self.offsets.get_unpack(vec_ix);
        let len = self.lengths.get_unpack(vec_ix);
        let iter = self.names.iter_slice(offset, len).view();

        Some(iter)
    }
}

#[derive(Debug, Clone)]
pub struct PackedGraphPaths {
    paths: Vec<PackedPath>,
    pub(super) path_props: PathProperties,
    pub(super) path_names: PackedPathNames,
}

impl Default for PackedGraphPaths {
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            path_props: Default::default(),
            path_names: Default::default(),
        }
    }
}

pub struct PathMutContext<'a> {
    path_ref_mut: PackedPathRefMut<'a>,
    path_properties: &'a mut PathProperties,
}

impl<'a> PathMutContext<'a> {
    pub(super) fn get_ref_mut<'b>(
        &'b mut self,
    ) -> &'b mut PackedPathRefMut<'a> {
        &mut self.path_ref_mut
    }
}

impl<'a> Drop for PathMutContext<'a> {
    fn drop(&mut self) {
        let path_id = self.path_ref_mut.path_id;
        let ix = path_id.0 as usize;
        let new_props = &self.path_ref_mut.properties;
        self.path_properties.heads.set_pack(ix, new_props.head);
        self.path_properties.tails.set_pack(ix, new_props.tail);
        self.path_properties
            .circular
            .set_pack(ix, new_props.circular);
        self.path_properties
            .deleted_steps
            .set_pack(ix, new_props.deleted_steps);
    }
}

impl<'a> PathBase for PathMutContext<'a> {
    type Step = (PathStepIx, PackedStep);

    type StepIx = PathStepIx;
}

impl<'a> PathRefMut for PathMutContext<'a> {
    fn append_step(&mut self, handle: Handle) -> StepUpdate {
        self.path_ref_mut.append_handle(handle)
    }

    fn prepend_step(&mut self, handle: Handle) -> StepUpdate {
        self.path_ref_mut.prepend_handle(handle)
    }

    fn set_circularity(&mut self, circular: bool) {
        self.path_ref_mut.properties.circular = circular;
    }
}

pub(super) struct MultiPathMutContext<'a> {
    paths: Vec<PackedPathRefMut<'a>>,
    path_properties: &'a mut PathProperties,
}

impl<'a> MultiPathMutContext<'a> {
    pub(super) fn get_ref_muts<'b>(
        &'b mut self,
    ) -> std::slice::IterMut<'b, PackedPathRefMut<'a>> {
        self.paths.iter_mut()
    }

    pub(super) fn ref_muts_par<'b>(
        &'b mut self,
    ) -> rayon::slice::IterMut<'b, PackedPathRefMut<'a>> {
        self.paths.par_iter_mut()
    }
}

impl<'a> Drop for MultiPathMutContext<'a> {
    fn drop(&mut self) {
        for path in self.paths.iter() {
            let path_id = path.path_id;
            let ix = path_id.0 as usize;
            let new_props = &path.properties;
            self.path_properties.heads.set_pack(ix, new_props.head);
            self.path_properties.tails.set_pack(ix, new_props.tail);
            self.path_properties
                .circular
                .set_pack(ix, new_props.circular);
            self.path_properties
                .deleted_steps
                .set_pack(ix, new_props.deleted_steps);
        }
    }
}

impl PackedGraphPaths {
    pub(super) fn create_path(&mut self, name: &[u8]) -> PathId {
        let path_id = self.paths.len() as u64;
        let packed_path = PackedPath::new();
        self.paths.push(packed_path);

        self.path_props.append_record();
        self.path_names.add_name(name);

        PathId(path_id)
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub(super) fn path_ref<'a>(
        &'a self,
        id: PathId,
    ) -> Option<PackedPathRef<'a>> {
        let path_id = id;
        let path = self.paths.get(id.0 as usize)?;
        let properties = self.path_props.get_record(id);
        Some(PackedPathRef::new(path_id, path, properties))
    }

    pub(super) fn path_properties_mut<'a>(
        &'a mut self,
        id: PathId,
    ) -> PathPropertyMut<'a> {
        self.path_props.record_mut(id)
    }

    pub(super) fn get_path_mut_ctx<'a>(
        &'a mut self,
        id: PathId,
    ) -> Option<PathMutContext<'a>> {
        let path_id = id;
        let path = self.paths.get_mut(id.0 as usize)?;
        let properties = self.path_props.get_record(id);
        let path_properties = &mut self.path_props;
        let path_ref_mut = PackedPathRefMut::new(path_id, path, properties);
        Some(PathMutContext {
            path_ref_mut,
            path_properties,
        })
    }

    pub(super) fn get_multipath_mut_ctx<'a>(
        &'a mut self,
    ) -> MultiPathMutContext<'a> {
        let mut_paths = &mut self.paths;
        let path_properties = &mut self.path_props;

        let paths = self
            .paths
            .iter_mut()
            .enumerate()
            .map(|(id, path)| {
                let path_id = PathId(id as u64);
                let properties = path_properties.get_record(path_id);
                PackedPathRefMut::new(path_id, path, properties)
            })
            .collect::<Vec<_>>();

        MultiPathMutContext {
            paths,
            path_properties,
        }
    }

    pub(super) fn with_path_mut_ctx<'a, F>(
        &'a mut self,
        id: PathId,
        mut f: F,
    ) -> Option<Vec<StepUpdate>>
    where
        F: Fn(&mut PackedPathRefMut<'a>) -> Vec<StepUpdate>,
    {
        let mut mut_ctx = self.get_path_mut_ctx(id)?;
        let mut ref_mut = mut_ctx.get_ref_mut();

        Some(f(ref_mut))
    }

    pub(super) fn with_multipath_mut_ctx<'a, F>(
        &'a mut self,
        mut f: F,
    ) -> Vec<(PathId, Vec<StepUpdate>)>
    where
        F: Fn(PathId, &mut PackedPathRefMut<'a>) -> Vec<StepUpdate>,
    {
        let mut mut_ctx = self.get_multipath_mut_ctx();
        let refs_mut = mut_ctx.get_ref_muts();

        let results = refs_mut
            .map(|path| {
                let path_id = path.path_id;
                let steps = f(path_id, path);
                (path_id, steps)
            })
            .collect::<Vec<_>>();

        results
    }
}

impl<'a> AllPathIds for &'a PackedGraphPaths {
    type PathIds = std::iter::Copied<
        std::collections::hash_map::Values<'a, Vec<u8>, PathId>,
    >;

    fn all_path_ids(self) -> Self::PathIds {
        self.path_names.name_id_map.values().copied()
    }
}

impl<'a> PathNames for &'a PackedPathNames {
    type PathName = packed::vector::IterView<'a, u8>;

    fn get_path_name(self, id: PathId) -> Option<Self::PathName> {
        self.name_iter(id)
    }

    fn get_path_id(self, name: &[u8]) -> Option<PathId> {
        self.name_id_map.get(name).copied()
    }
}

impl<'a> PathNames for &'a PackedGraphPaths {
    type PathName = packed::vector::IterView<'a, u8>;

    fn get_path_name(self, id: PathId) -> Option<Self::PathName> {
        self.path_names.name_iter(id)
    }

    fn get_path_id(self, name: &[u8]) -> Option<PathId> {
        self.path_names.name_id_map.get(name).copied()
    }
}

impl<'a> PathNamesMut for &'a mut PackedGraphPaths {
    fn insert_name(self, name: &[u8]) -> Option<PathId> {
        if self.get_path_id(name).is_some() {
            None
        } else {
            Some(self.path_names.add_name(name))
        }
    }
}

impl<'a> PathRefs for &'a PackedGraphPaths {
    type Path = PackedPathRef<'a>;

    fn path_ref(self, id: PathId) -> Option<PackedPathRef<'a>> {
        self.path_ref(id)
    }
}

impl<'a> PathRefsMut for &'a mut PackedGraphPaths {
    type PathMut = PathMutContext<'a>;

    fn path_ref_mut(self, id: PathId) -> Option<PathMutContext<'a>> {
        self.get_path_mut_ctx(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packedpathpaths_new_path() {
        let mut p_path = PackedPath::new();

        let hnd = |x: u64| Handle::pack(x, false);

        let s1 = p_path.append_handle(hnd(1));
        let s2 = p_path.insert_after(s1, hnd(4)).unwrap();
        let s3 = p_path.insert_after(s2, hnd(3)).unwrap();
        let s4 = p_path.insert_after(s3, hnd(2)).unwrap();

        let steps_fwd = p_path
            .iter(s1, PathStepIx::null())
            .map(|(ix, step)| {
                (ix.to_vector_value(), u64::from(step.handle.id()))
            })
            .collect::<Vec<_>>();

        assert_eq!(steps_fwd, vec![(1, 1), (2, 4), (3, 3), (4, 2)]);

        let steps_bwd = p_path
            .iter(PathStepIx::null(), s4)
            .rev()
            .map(|(ix, step)| {
                (ix.to_vector_value(), u64::from(step.handle.id()))
            })
            .collect::<Vec<_>>();

        assert_eq!(steps_bwd, vec![(4, 2), (3, 3), (2, 4), (1, 1)]);

        let s5 = p_path.insert_before(s3, hnd(5)).unwrap();
        let s6 = p_path.insert_before(s1, hnd(6)).unwrap();

        let steps_fwd = p_path
            .iter(s6, PathStepIx::null())
            .map(|(ix, step)| {
                (ix.to_vector_value(), u64::from(step.handle.id()))
            })
            .collect::<Vec<_>>();

        assert_eq!(
            steps_fwd,
            vec![(6, 6), (1, 1), (2, 4), (5, 5), (3, 3), (4, 2)]
        );

        let steps_bwd = p_path
            .iter(PathStepIx::null(), s4)
            .rev()
            .map(|(ix, step)| {
                (ix.to_vector_value(), u64::from(step.handle.id()))
            })
            .collect::<Vec<_>>();

        assert_eq!(
            steps_bwd,
            vec![(4, 2), (3, 3), (5, 5), (2, 4), (1, 1), (6, 6)]
        );
    }

    #[test]
    fn packedgraphpaths_paths_add() {
        let hnd = |x: u64| Handle::pack(x, false);

        let mut paths = PackedGraphPaths::default();

        let path_1 = paths.create_path(b"path1");

        let pre_record = paths.path_props.get_record(path_1);

        assert!(pre_record.head.is_null());
        assert!(pre_record.tail.is_null());

        let _step_updates = {
            let mut path_mut = paths.get_path_mut_ctx(path_1).unwrap();
            let ref_mut = path_mut.get_ref_mut();

            let steps = vec![1, 2, 3, 4, 3, 5]
                .into_iter()
                .map(|n| {
                    let h = hnd(n);
                    let s = ref_mut.append_handle(h);
                    s
                })
                .collect::<Vec<_>>();

            steps
        };

        let post_record = paths.path_props.get_record(path_1);

        // Heads & tails are path step indices, not handles
        assert_eq!(post_record.head.to_vector_value(), 1);
        assert_eq!(post_record.tail.to_vector_value(), 6);

        let path_ref = paths.path_ref(path_1).unwrap();

        // PackedPathRef implements the PathRef trait,
        // so we can step through the path
        let steps = path_ref
            .steps()
            .map(|(_ix, step)| step.handle)
            .collect::<Vec<_>>();

        let mut expected_steps =
            vec![hnd(1), hnd(2), hnd(3), hnd(4), hnd(3), hnd(5)];
        assert_eq!(steps, expected_steps);

        // The step iterator is double-ended, so we can reverse it
        let steps_rev = path_ref
            .steps()
            .rev()
            .map(|(_ix, step)| step.handle)
            .collect::<Vec<_>>();

        expected_steps.reverse();
        assert_eq!(steps_rev, expected_steps);
    }

    #[test]
    fn packedgraphpaths_path_with_mut_ctx() {
        let hnd = |x: u64| Handle::pack(x, false);

        let mut paths = PackedGraphPaths::default();

        let path_1 = paths.create_path(b"path1");

        let _steps = paths.with_path_mut_ctx(path_1, |ref_mut| {
            vec![1, 2, 3, 4, 3, 5]
                .into_iter()
                .map(|n| ref_mut.append_handle(hnd(n)))
                .collect::<Vec<_>>()
        });

        let post_record = paths.path_props.get_record(path_1);
        assert_eq!(post_record.head.to_vector_value(), 1);
        assert_eq!(post_record.tail.to_vector_value(), 6);

        let path_ref = paths.path_ref(path_1).unwrap();

        let steps = path_ref
            .steps()
            .map(|(_ix, step)| step.handle)
            .collect::<Vec<_>>();

        let mut expected_steps =
            vec![hnd(1), hnd(2), hnd(3), hnd(4), hnd(3), hnd(5)];
        assert_eq!(steps, expected_steps);
    }

    #[test]
    fn packedgraphpaths_multipaths() {
        let hnd = |x: u64| Handle::pack(x, false);

        let mut paths = PackedGraphPaths::default();

        let path_1 = paths.create_path(b"path1");
        let path_2 = paths.create_path(b"path2");
        let path_3 = paths.create_path(b"path3");

        let vec_hnd = |v: Vec<u64>| v.into_iter().map(hnd).collect::<Vec<_>>();

        // Path 1: 1 2 3 4 5
        // Path 2: 6 2 3 7 5
        // Path 3: 1 6 2 3 4

        let pre_1 = paths.path_props.get_record(path_1);
        let pre_2 = paths.path_props.get_record(path_2);
        let pre_3 = paths.path_props.get_record(path_3);

        assert!(pre_1.head.is_null() && pre_1.tail.is_null());
        assert!(pre_2.head.is_null() && pre_2.tail.is_null());
        assert!(pre_3.head.is_null() && pre_3.tail.is_null());

        let to_insert_1 = vec_hnd(vec![1, 2, 3, 4, 5]);
        let to_insert_2 = vec_hnd(vec![6, 2, 3, 7, 5]);
        let to_insert_3 = vec_hnd(vec![1, 6, 2, 3, 4]);

        let to_insert = vec![
            to_insert_1.clone(),
            to_insert_2.clone(),
            to_insert_3.clone(),
        ];

        let _step_updates = {
            let mut mut_ctx = paths.get_multipath_mut_ctx();
            let paths_mut = mut_ctx.get_ref_muts();

            paths_mut
                .zip(to_insert)
                .map(|(path, steps)| {
                    steps
                        .into_iter()
                        .map(|h| path.append_handle(h))
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };

        let post_1 = paths.path_props.get_record(path_1);
        let post_2 = paths.path_props.get_record(path_2);
        let post_3 = paths.path_props.get_record(path_3);

        assert_eq!(post_1.head.to_vector_value(), 1);
        assert_eq!(post_1.tail.to_vector_value(), 5);

        assert_eq!(post_2.head.to_vector_value(), 1);
        assert_eq!(post_2.tail.to_vector_value(), 5);

        assert_eq!(post_3.head.to_vector_value(), 1);
        assert_eq!(post_3.tail.to_vector_value(), 5);

        let ref_1 = paths.path_ref(path_1).unwrap();
        let ref_2 = paths.path_ref(path_2).unwrap();
        let ref_3 = paths.path_ref(path_3).unwrap();

        let get_steps = |path_ref: PackedPathRef<'_>| {
            path_ref
                .steps()
                .map(|(_ix, step)| step.handle)
                .collect::<Vec<_>>()
        };

        assert_eq!(get_steps(ref_1), to_insert_1);
        assert_eq!(get_steps(ref_2), to_insert_2);
        assert_eq!(get_steps(ref_3), to_insert_3);
    }
}
