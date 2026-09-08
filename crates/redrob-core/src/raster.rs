// SPDX-License-Identifier: GPL-3.0-or-later

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Serializable, clone-on-write byte storage for potentially large raster data.
///
/// Cloning this value is constant-time and shares its allocation. The allocation
/// is copied only when mutable access is requested. Its JSON representation is
/// the same byte array used by `Vec<u8>`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RasterBytes(Arc<Vec<u8>>);

impl RasterBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Arc::new(bytes))
    }

    pub fn zeroed(len: usize) -> Self {
        Self::new(vec![0; len])
    }

    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.as_ref().clone()
    }

    /// Returns whether two values share the same backing allocation.
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn allocation_bytes(&self) -> usize {
        self.0.capacity()
    }

    pub(crate) fn allocation_identity(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
}

impl From<Vec<u8>> for RasterBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl AsRef<[u8]> for RasterBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Deref for RasterBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl DerefMut for RasterBytes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0).as_mut_slice()
    }
}

impl Serialize for RasterBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RasterBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<u8>::deserialize(deserializer).map(Self::new)
    }
}
