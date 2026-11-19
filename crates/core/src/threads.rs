//! Work spread over every core.

use std::sync::atomic::{AtomicUsize, Ordering};

/// `f` of every item, on all cores.
pub(crate) fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(items.len());
    if threads <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let mut out: Vec<(usize, R)> = std::thread::scope(|s| {
        let work = || {
            let mut mine = vec![];
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(i) else { return mine };
                mine.push((i, f(item)));
            }
        };
        let threads: Vec<_> = (0..threads).map(|_| s.spawn(work)).collect();
        threads.into_iter().flat_map(|t| t.join().unwrap()).collect()
    });
    out.sort_unstable_by_key(|r| r.0);
    out.into_iter().map(|r| r.1).collect()
}
