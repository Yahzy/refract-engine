//! Entities: things of the world with an id that stays valid while they
//! exist and is never confused with a later one in the same slot, so it can
//! be sent over the network or kept by scripts.

/// An index and the generation of its slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId {
    index: u32,
    generation: u32,
}

impl EntityId {
    /// As one number, generation in the high half.
    pub fn to_bits(self) -> u64 {
        (self.generation as u64) << 32 | self.index as u64
    }

    pub fn from_bits(bits: u64) -> Self {
        Self { index: bits as u32, generation: (bits >> 32) as u32 }
    }

    pub fn index(self) -> u32 {
        self.index
    }
}

/// Values stored under entity ids. Removing frees the slot for reuse
/// with the next generation.
#[derive(Clone, Debug)]
pub struct Entities<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    len: usize,
}

#[derive(Clone, Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

impl<T> Default for Entities<T> {
    fn default() -> Self {
        Self { slots: Vec::new(), free: Vec::new(), len: 0 }
    }
}

impl<T> Entities<T> {
    pub fn insert(&mut self, value: T) -> EntityId {
        self.len += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            return EntityId { index, generation: slot.generation };
        }
        let index = self.slots.len() as u32;
        self.slots.push(Slot { generation: 0, value: Some(value) });
        EntityId { index, generation: 0 }
    }

    pub fn remove(&mut self, id: EntityId) -> Option<T> {
        let slot = self.slots.get_mut(id.index as usize).filter(|slot| slot.generation == id.generation)?;
        let value = slot.value.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.index);
        self.len -= 1;
        Some(value)
    }

    pub fn get(&self, id: EntityId) -> Option<&T> {
        self.slots.get(id.index as usize).filter(|slot| slot.generation == id.generation)?.value.as_ref()
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut T> {
        self.slots.get_mut(id.index as usize).filter(|slot| slot.generation == id.generation)?.value.as_mut()
    }

    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &T)> {
        self.slots.iter().enumerate().filter_map(|(index, slot)| Some((EntityId { index: index as u32, generation: slot.generation }, slot.value.as_ref()?)))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut T)> {
        self.slots.iter_mut().enumerate().filter_map(|(index, slot)| Some((EntityId { index: index as u32, generation: slot.generation }, slot.value.as_mut()?)))
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_ids_stay_dead_when_the_slot_is_reused() {
        let mut entities = Entities::default();
        let a = entities.insert("a");
        let b = entities.insert("b");
        assert_eq!(entities.remove(a), Some("a"));
        assert_eq!(entities.get(a), None);
        let c = entities.insert("c");
        assert_eq!(c.index(), a.index());
        assert_ne!(c, a);
        assert_eq!(entities.get(a), None);
        assert_eq!(entities.remove(a), None);
        assert_eq!(entities.get(c), Some(&"c"));
        assert_eq!(entities.iter().map(|(id, v)| (id, *v)).collect::<Vec<_>>(), vec![(c, "c"), (b, "b")]);
        assert_eq!(entities.len(), 2);
        assert_eq!(EntityId::from_bits(c.to_bits()), c);
    }
}
