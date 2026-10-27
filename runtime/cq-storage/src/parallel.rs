//! Index-based task dispatch shared by generated and native operators.

use rayon::prelude::*;
use std::sync::Mutex;

/// Invoke `body` exactly once per index, joining all tasks before returning.
/// Empty and singleton ranges do not enter the task pool.
pub fn for_each_index(count: usize, body: &(dyn Fn(usize) + Sync)) {
    match count {
        0 => {}
        1 => body(0),
        _ => (0..count).into_par_iter().for_each(body),
    }
}

/// Map indices in parallel and return results in index order.
/// Each slot owns its result, so results need `Send` but not `Sync`.
pub fn map_index<T: Send>(count: usize, body: &(dyn Fn(usize) -> T + Sync)) -> Vec<T> {
    match count {
        0 => return Vec::new(),
        1 => return vec![body(0)],
        _ => {}
    }
    let slots: Vec<_> = (0..count).map(|_| Mutex::new(None)).collect();
    for_each_index(count, &|index| {
        // Run user code outside the lock, including its possible unwind.
        let value = body(index);
        *slots[index].lock().unwrap() = Some(value);
    });
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap()
                .expect("index task produced a result")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn visits_every_index_exactly_once() {
        for count in [0, 1, 257] {
            let visits: Vec<_> = (0..count).map(|_| AtomicUsize::new(0)).collect();
            for_each_index(count, &|index| {
                visits[index].fetch_add(1, Ordering::Relaxed);
            });
            assert!(
                visits
                    .iter()
                    .all(|visit| visit.load(Ordering::Relaxed) == 1)
            );
        }
    }

    #[test]
    fn maps_send_non_sync_results_in_index_order() {
        for count in [0, 1, 257] {
            let calls = AtomicUsize::new(0);
            let results = map_index(count, &|index| {
                calls.fetch_add(1, Ordering::Relaxed);
                Cell::new(index)
            });
            assert_eq!(calls.load(Ordering::Relaxed), count);
            assert_eq!(
                results
                    .into_iter()
                    .map(Cell::into_inner)
                    .collect::<Vec<_>>(),
                (0..count).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn singleton_runs_on_caller() {
        let caller = std::thread::current().id();
        for_each_index(1, &|index| {
            assert_eq!(index, 0);
            assert_eq!(std::thread::current().id(), caller);
        });
        assert_eq!(map_index(1, &|_| std::thread::current().id()), [caller]);
    }

    #[test]
    fn panic_drops_every_produced_result() {
        struct Tracked<'a>(&'a AtomicUsize, Cell<usize>);
        impl Drop for Tracked<'_> {
            fn drop(&mut self) {
                self.0.fetch_add(self.1.get(), Ordering::Relaxed);
            }
        }
        let produced = AtomicUsize::new(0);
        let dropped = AtomicUsize::new(0);
        let result = std::panic::catch_unwind(|| {
            map_index(257, &|index| {
                produced.fetch_add(1, Ordering::Relaxed);
                let value = Tracked(&dropped, Cell::new(1));
                assert_ne!(index, 128, "task panic");
                value
            })
        });
        assert!(result.is_err());
        assert!(produced.load(Ordering::Relaxed) > 0);
        assert_eq!(
            produced.load(Ordering::Relaxed),
            dropped.load(Ordering::Relaxed)
        );
    }
}
