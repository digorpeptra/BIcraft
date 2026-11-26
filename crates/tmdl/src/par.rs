//! Spreading file system work over threads. On Windows each file open costs far more than its
//! bytes (on-access scanning, NTFS), so overlapping the opens of a model's many small files is
//! what makes reading and writing a TMDL folder fast there (docs/benchmarks.md, G23).

/// Fewer items than this are handled on the calling thread: starting threads costs more than it
/// saves.
const PARALLEL_MIN_ITEMS: usize = 16;
/// Most worker threads.
const MAX_WORKERS: usize = 16;

/// `items.iter().map(f)`, in order, spread over up to [`MAX_WORKERS`] threads in contiguous
/// chunks. When a thread cannot be started (or fails) its chunk runs on the calling thread.
pub(crate) fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = if items.len() < PARALLEL_MIN_ITEMS {
        1
    } else {
        std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .min(MAX_WORKERS)
    };
    if workers <= 1 {
        return items.iter().map(&f).collect();
    }
    let chunk = items.len().div_ceil(workers).max(1);
    let f = &f;
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| {
                let handle = std::thread::Builder::new()
                    .spawn_scoped(scope, move || part.iter().map(f).collect::<Vec<R>>())
                    .ok();
                (part, handle)
            })
            .collect();
        let mut out = Vec::with_capacity(items.len());
        for (part, handle) in handles {
            match handle.map(std::thread::ScopedJoinHandle::join) {
                Some(Ok(results)) => out.extend(results),
                _ => out.extend(part.iter().map(f)),
            }
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_order() {
        let items: Vec<usize> = (0..1000).collect();
        let doubled: Vec<usize> = items.iter().map(|x| x * 2).collect();
        assert_eq!(parallel_map(&items, |x| x * 2), doubled);
        assert_eq!(parallel_map(&items[..3], |x| *x), [0, 1, 2]);
        assert!(parallel_map(&items[..0], |x| *x).is_empty());
    }
}
