//! Typed indices into the dotgen arenas (technique T5, `docs/roadmap/post-1.0.md`).
//!
//! C's dotgen passes `node_t *`, `edge_t *`, `graph_t *` and `textlabel_t *`
//! around; the port addresses the [`super::Dot`] arenas by index instead, and
//! these newtypes keep the four index kinds ([`N`], [`E`], [`G`], [`L`]) apart.
//! Each is a `NonZeroU32` of `index + 1`, so an `Option<NodeIx>` is 4 bytes.
//!
//! [`N`]: super::types::N
//! [`E`]: super::types::E
//! [`G`]: super::types::G
//! [`L`]: super::types::L

use std::marker::PhantomData;
use std::num::NonZeroU32;
use std::ops::{Index, IndexMut};

/// A typed arena index: one of [`NodeIx`], [`EdgeIx`], [`GraphIx`], [`LabelIx`].
pub trait Ix: Copy + Eq + Ord + std::hash::Hash {
    /// The index of arena slot `ix`.
    fn from_index(ix: usize) -> Self;
    /// The arena slot this index addresses.
    fn index(self) -> usize;
}

macro_rules! ix_newtype {
    ($name:ident, $what:literal) => {
        #[doc = concat!("Index of ", $what, " (a `NonZeroU32` of `index + 1`, so `Option<", stringify!($name), ">` is 4 bytes).")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU32);

        impl Ix for $name {
            #[inline]
            fn from_index(ix: usize) -> Self {
                match u32::try_from(ix).ok().and_then(|i| NonZeroU32::new(i.wrapping_add(1))) {
                    Some(ix) => Self(ix),
                    // Unreachable: the arena would need u32::MAX (or more) slots.
                    None => panic!(concat!(stringify!($name), " arena index overflow")),
                }
            }

            #[inline]
            fn index(self) -> usize {
                self.0.get().wrapping_sub(1) as usize
            }
        }

        impl From<usize> for $name {
            #[inline]
            fn from(ix: usize) -> Self {
                Self::from_index(ix)
            }
        }

        impl From<$name> for usize {
            #[inline]
            fn from(ix: $name) -> Self {
                ix.index()
            }
        }
    };
}

ix_newtype!(NodeIx, "a node in `Dot::nodes` (`node_t *`)");
ix_newtype!(EdgeIx, "an edge in `Dot::edges` (`edge_t *`)");
ix_newtype!(GraphIx, "a graph in `Dot::graphs` (`graph_t *`)");
ix_newtype!(LabelIx, "a label in `Dot::labels` (`textlabel_t *`)");

/// A `Vec` addressed by a typed [`Ix`]: an `IndexVec<NodeIx, NodeInfo>` cannot
/// be indexed with an `EdgeIx`. Outside the panicking indexing operations
/// (which fire on the same out-of-bounds indices C's pointers would turn into
/// undefined behaviour), it is a plain `Vec`.
#[derive(Debug, Clone, Default)]
pub struct IndexVec<I, T> {
    raw: Vec<T>,
    _ix: PhantomData<fn(I) -> I>,
}

impl<I, T> IndexVec<I, T> {
    pub fn new() -> Self {
        IndexVec {
            raw: Vec::new(),
            _ix: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.raw.len()
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.raw.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.raw.iter_mut()
    }
}

impl<I: Ix, T> IndexVec<I, T> {
    /// Appends `value` and returns its index.
    #[inline]
    pub fn push(&mut self, value: T) -> I {
        let ix = I::from_index(self.raw.len());
        self.raw.push(value);
        ix
    }

    pub fn get(&self, ix: I) -> Option<&T> {
        self.raw.get(ix.index())
    }

    pub fn get_mut(&mut self, ix: I) -> Option<&mut T> {
        self.raw.get_mut(ix.index())
    }
}

impl<I: Ix, T> Index<I> for IndexVec<I, T> {
    type Output = T;

    #[inline]
    fn index(&self, ix: I) -> &T {
        &self.raw[ix.index()]
    }
}

impl<I: Ix, T> IndexMut<I> for IndexVec<I, T> {
    #[inline]
    fn index_mut(&mut self, ix: I) -> &mut T {
        &mut self.raw[ix.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_of_an_ix_is_four_bytes() {
        assert_eq!(std::mem::size_of::<Option<NodeIx>>(), 4);
        assert_eq!(std::mem::size_of::<Option<EdgeIx>>(), 4);
        assert_eq!(std::mem::size_of::<Option<GraphIx>>(), 4);
        assert_eq!(std::mem::size_of::<Option<LabelIx>>(), 4);
    }

    #[test]
    fn round_trips_through_usize() {
        for ix in [0usize, 1, 7, u32::MAX as usize - 1] {
            assert_eq!(usize::from(NodeIx::from_index(ix)), ix);
            assert_eq!(usize::from(LabelIx::from(ix)), ix);
        }
    }

    #[test]
    #[should_panic]
    fn an_index_past_u32_panics_instead_of_aliasing_slot_0() {
        let _ = NodeIx::from_index(1usize << 32);
    }

    #[test]
    fn orders_and_hashes_like_the_underlying_index() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        assert!(NodeIx::from_index(3) < NodeIx::from_index(4));
        assert_ne!(EdgeIx::from_index(3), EdgeIx::from_index(4));
        let hash = |ix: EdgeIx| {
            let mut h = DefaultHasher::new();
            ix.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash(EdgeIx::from_index(5)), hash(EdgeIx::from_index(5)));
    }

    #[test]
    fn push_returns_indices_that_index_back() {
        let mut v: IndexVec<LabelIx, u8> = IndexVec::new();
        assert!(v.is_empty());
        let a = v.push(10);
        let b = v.push(20);
        assert_eq!(v.len(), 2);
        assert_eq!(v[a], 10);
        assert_eq!(v[b], 20);
        assert_eq!(v.get(b), Some(&20));
        assert_eq!(v.get(LabelIx::from_index(2)), None);
        v[b] = 21;
        *v.get_mut(a).unwrap() = 11;
        assert_eq!(v.iter().copied().collect::<Vec<_>>(), [11, 21]);
        v.iter_mut().for_each(|x| *x += 1);
        assert_eq!(v.iter().copied().collect::<Vec<_>>(), [12, 22]);
    }

    #[test]
    #[should_panic]
    fn indexing_out_of_bounds_panics() {
        let v: IndexVec<GraphIx, u8> = IndexVec::new();
        let _ = &v[GraphIx::from_index(0)];
    }
}
