//! Copy-on-write `Arc` wrapper for shared layer payloads (`TextData`, `VectorNetwork`).
//!
//! Cloning a [`Shared`] payload is an O(1) atomic reference-count bump, so mirrored layers inside
//! component instances and duplicated subtrees share identical vector networks and text data in
//! memory. Mutating through [`DerefMut`] detaches via [`Arc::make_mut`], leaving every other
//! holder unchanged. Serialization and deserialization are transparent.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Copy-on-write reference-counted wrapper around `T`.
#[derive(Debug, Default)]
pub struct Shared<T>(Arc<T>);

impl<T> Shared<T> {
    /// Wrap `value` in a new shared allocation.
    pub fn new(value: T) -> Self {
        Self(Arc::new(value))
    }

    /// Wrap an existing [`Arc`].
    pub fn from_arc(arc: Arc<T>) -> Self {
        Self(arc)
    }

    /// Borrow the underlying [`Arc`].
    pub fn as_arc(&self) -> &Arc<T> {
        &self.0
    }

    /// Unwrap the underlying [`Arc`].
    pub fn into_arc(self) -> Arc<T> {
        self.0
    }

    /// Whether `self` and `other` point to the exact same allocation.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Current strong reference count of the underlying allocation.
    pub fn strong_count(&self) -> usize {
        Arc::strong_count(&self.0)
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Deref for Shared<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: Clone> DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut T {
        Arc::make_mut(&mut self.0)
    }
}

impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        *self.0 == *other.0
    }
}

impl<T: Eq> Eq for Shared<T> {}

impl<T> From<T> for Shared<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T> From<Arc<T>> for Shared<T> {
    fn from(arc: Arc<T>) -> Self {
        Self(arc)
    }
}

impl<T> AsRef<T> for Shared<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

impl<T: Serialize> Serialize for Shared<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Shared<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::deserialize(deserializer).map(Self::new)
    }
}
