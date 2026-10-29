//! Hashing for the in-memory tables of values (value index, distinct
//! counts). The data is the user's own file, so the default SipHash, which
//! guards against crafted collisions, is not needed and costs speed.

use std::collections::{HashMap, HashSet};

pub(crate) type State = rapidhash::fast::RandomState;
pub(crate) type Map<K, V> = HashMap<K, V, State>;
pub(crate) type Set<K> = HashSet<K, State>;

/// A 64-bit fingerprint of a value's bytes, standing in for the value when
/// counting without an index.
pub(crate) fn fingerprint(bytes: &[u8]) -> u64 {
    rapidhash::v3::rapidhash_v3(bytes)
}
