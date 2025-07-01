use std::{collections::VecDeque, sync::atomic::AtomicU32};

use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct Index {
    index: u32,
    generation: u32,
}

struct IndexAllocator {
    next_index: AtomicU32,
    // TODO: Use channel instead if mutable access will cause problems
    recycle_queue: VecDeque<Index>,
    recycled: Vec<Index>,
}

impl IndexAllocator {
    fn reserve(&mut self) -> Index {
        if let Some(mut recycled) = self.recycle_queue.pop_front() {
            recycled.generation += 1;
            self.recycled.push(recycled);
            recycled
        } else {
            Index {
                index: self
                    .next_index
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                generation: 0,
            }
        }
    }

    fn recycle(&mut self, index: Index) {
        self.recycle_queue.push_back(index);
    }
}

struct Entry<T> {
    value: Option<T>,
    generation: u32,
}

pub struct DenseStorage<T> {
    buffer: Vec<Entry<T>>,
    len: u32,
    index_allocator: IndexAllocator,
}

impl<T> DenseStorage<T> {
    /// Returns the number of stored items.
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Returns `true` if there's no items stored.
    pub fn is_emty(&self) -> bool {
        self.len() == 0
    }

    pub fn insert(&mut self, index: Index, value: T) -> Result<bool, InvalidGenerationError> {
        self.flush();
        let entry = &mut self.buffer[index.index as usize];
        if entry.generation == index.generation {
            let exists = entry.value.is_some();
            // If it didn't exists that means we're adding a new item.
            if !exists {
                self.len += 1;
            }
            entry.value = Some(value);
            Ok(exists)
        } else {
            Err(InvalidGenerationError {
                index,
                current_generation: entry.generation,
            })
        }
    }

    fn flush(&mut self) {
        let new_len = self
            .index_allocator
            .next_index
            .load(std::sync::atomic::Ordering::Relaxed);

        self.buffer.resize_with(new_len as usize, || Entry {
            value: None,
            generation: 0,
        });

        for index in self.index_allocator.recycled.drain(..) {
            let entry = &mut self.buffer[index.index as usize];
            *entry = Entry {
                value: None,
                generation: index.generation,
            }
        }
    }
}

#[derive(Error, Debug)]
#[error("{index:?} has invalid generation. Current generation is {current_generation}")]
pub struct InvalidGenerationError {
    index: Index,
    current_generation: u32,
}
