use std::collections::HashMap;
use std::hash::Hash;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InternId(u32);

impl InternId {
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

#[derive(Debug)]
pub struct Interner<K> {
    map: HashMap<K, InternId>,
    keys: Vec<K>,
}

impl<K> Default for Interner<K> {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
            keys: Vec::new(),
        }
    }
}

impl<K> Interner<K>
where
    K: Eq + Hash + Clone,
{
    pub fn intern(&mut self, key: K) -> InternId {
        if let Some(id) = self.map.get(&key) {
            return *id;
        }

        let id = InternId::from_raw(self.keys.len() as u32);
        self.keys.push(key.clone());
        self.map.insert(key, id);
        id
    }

    pub fn lookup(&self, id: InternId) -> Option<&K> {
        self.keys.get(id.index() as usize)
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::Interner;

    #[test]
    fn interning_returns_stable_ids() {
        let mut interner = Interner::default();
        let first = interner.intern("foo");
        let second = interner.intern("foo");
        assert_eq!(first, second);
        assert_eq!(interner.lookup(first), Some(&"foo"));
    }
}
