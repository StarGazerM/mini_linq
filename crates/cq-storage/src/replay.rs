//! Independent cursors over one incrementally retained iterator sequence.

use std::{cell::RefCell, iter::FusedIterator, rc::Rc};

/// Read one stable sequence through independently positioned, owning cursors.
///
/// Clones share the same sequence and its retained prefix; they must not restart
/// the upstream computation. Constructing a cursor performs no upstream poll.
/// Each cursor starts at position zero, preserves occurrence order and
/// multiplicity, and remains usable after the handle which created it is
/// dropped. Items are copied values, commonly tuples of references into storage
/// owned by the caller; cursors do not borrow an internal growable buffer.
pub trait ReplayRead: Clone {
    type Item: Copy;
    type Cursor: Iterator<Item = Self::Item>;

    fn cursor(&self) -> Self::Cursor;
}

/// Retain an iterator without advancing it during construction.
///
/// Cursors replay cached occurrences. A cursor at the retained frontier polls
/// the one source for its next occurrence and appends any produced item. The
/// first `None` permanently ends the sequence and releases the source; later
/// cursors can still read the retained prefix without polling it again.
pub trait BuildReplay<I>: ReplayRead<Item = I::Item>
where
    I: Iterator,
    I::Item: Copy,
{
    fn replay(source: I) -> Self;
}

/// A shared, lazily retained sequence with no relational or query operations.
///
/// Only the demanded prefix is retained. It has no eviction policy and can grow
/// to the complete sequence. The shared state uses `Rc` and is not `Send`.
/// Advancing its source must not re-enter the same replay state.
pub struct Replay<I: Iterator> {
    state: Rc<RefCell<State<I>>>,
}

struct State<I: Iterator> {
    source: Option<I>,
    prefix: Vec<I::Item>,
}

// Deriving Clone would unnecessarily require the source iterator to be Clone.
impl<I: Iterator> Clone for Replay<I> {
    fn clone(&self) -> Self {
        Self {
            state: Rc::clone(&self.state),
        }
    }
}

impl<I> BuildReplay<I> for Replay<I>
where
    I: Iterator,
    I::Item: Copy,
{
    fn replay(source: I) -> Self {
        Self {
            state: Rc::new(RefCell::new(State {
                source: Some(source),
                prefix: Vec::new(),
            })),
        }
    }
}

impl<I> ReplayRead for Replay<I>
where
    I: Iterator,
    I::Item: Copy,
{
    type Item = I::Item;
    type Cursor = ReplayCursor<I>;

    fn cursor(&self) -> Self::Cursor {
        ReplayCursor {
            state: Rc::clone(&self.state),
            position: 0,
        }
    }
}

/// One independent traversal which owns a share of its replay state.
pub struct ReplayCursor<I: Iterator> {
    state: Rc<RefCell<State<I>>>,
    position: usize,
}

impl<I> Iterator for ReplayCursor<I>
where
    I: Iterator,
    I::Item: Copy,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let mut state = self.state.borrow_mut();
        let item = if self.position < state.prefix.len() {
            // RP-Hit: replay a retained occurrence without polling the source.
            Some(state.prefix[self.position])
        } else {
            let item = state.source.as_mut().and_then(Iterator::next);
            if let Some(item) = item {
                // RP-Append: extend the shared prefix by one source occurrence.
                state.prefix.push(item);
            } else {
                // RP-End: release the source permanently at its first exhaustion.
                state.source = None;
            }
            item
        };
        if item.is_some() {
            self.position += 1;
        }
        item
    }
}

impl<I> FusedIterator for ReplayCursor<I>
where
    I: Iterator,
    I::Item: Copy,
{
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    // Deliberately not Clone: sharing a replay must never clone this source.
    struct Tracked<I> {
        source: I,
        polls: Rc<Cell<usize>>,
        drops: Rc<Cell<usize>>,
    }

    impl<I: Iterator> Iterator for Tracked<I> {
        type Item = I::Item;

        fn next(&mut self) -> Option<Self::Item> {
            self.polls.set(self.polls.get() + 1);
            self.source.next()
        }
    }

    impl<I> Drop for Tracked<I> {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    fn tracked<I>(source: I) -> (Tracked<I>, Rc<Cell<usize>>, Rc<Cell<usize>>) {
        let polls = Rc::new(Cell::new(0));
        let drops = Rc::new(Cell::new(0));
        (
            Tracked {
                source,
                polls: Rc::clone(&polls),
                drops: Rc::clone(&drops),
            },
            polls,
            drops,
        )
    }

    #[test]
    fn interleaved_cursors_extend_one_shared_frontier() {
        let (source, polls, drops) = tracked([1, 1, 2].into_iter());
        let replay = Replay::replay(source);
        let shared = replay.clone();
        let mut a = replay.cursor();
        let mut b = shared.cursor();
        assert_eq!(polls.get(), 0);

        assert_eq!(a.next(), Some(1));
        assert_eq!(b.next(), Some(1));
        assert_eq!(polls.get(), 1);
        assert_eq!(b.next(), Some(1));
        assert_eq!(a.next(), Some(1));
        assert_eq!(polls.get(), 2);
        assert_eq!(b.next(), Some(2));
        assert_eq!(a.next(), Some(2));
        assert_eq!(polls.get(), 3);
        assert_eq!(a.next(), None);
        assert_eq!(b.next(), None);
        assert_eq!(a.next(), None);
        assert_eq!(polls.get(), 4);
        assert_eq!(drops.get(), 1);
        assert_eq!(replay.cursor().collect::<Vec<_>>(), vec![1, 1, 2]);
        assert_eq!(polls.get(), 4);
    }

    #[test]
    fn a_cursor_outlives_its_handle_and_drop_does_not_drain() {
        let (source, polls, drops) = tracked([10, 20, 30].into_iter());
        let replay = Replay::replay(source);
        let mut cursor = replay.cursor();
        drop(replay);
        assert_eq!((polls.get(), drops.get()), (0, 0));
        assert_eq!(cursor.next(), Some(10));
        assert_eq!((polls.get(), drops.get()), (1, 0));
        drop(cursor);
        assert_eq!((polls.get(), drops.get()), (1, 1));
    }

    #[test]
    fn exhaustion_releases_even_a_source_that_would_resume() {
        let mut next = 0;
        let source = std::iter::from_fn(move || {
            next += 1;
            match next {
                1 => Some(7),
                2 => None,
                _ => Some(99),
            }
        });
        let (source, polls, drops) = tracked(source);
        let replay = Replay::replay(source);
        assert_eq!(replay.cursor().collect::<Vec<_>>(), vec![7]);
        assert_eq!((polls.get(), drops.get()), (2, 1));
        assert_eq!(replay.cursor().collect::<Vec<_>>(), vec![7]);
        assert_eq!((polls.get(), drops.get()), (2, 1));
    }

    #[test]
    fn retained_references_need_no_payload_clone_and_survive_cache_drop() {
        struct Payload(String);
        let values = [Payload("first".into()), Payload("second".into())];
        let replay = Replay::replay(values.iter().map(|value| (value,)));
        let first = replay.cursor().next().unwrap().0;
        let second = replay.clone().cursor().nth(1).unwrap().0;
        assert!(std::ptr::eq(first, &values[0]));
        assert!(std::ptr::eq(second, &values[1]));
        drop(replay);
        assert_eq!(first.0, "first");
        assert_eq!(second.0, "second");
    }
}
