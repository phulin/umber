//! Order-preserving data parallelism for independent finalization work.
//!
//! Page lowering and stream compression are pure per-item functions. Native
//! hosts spread contiguous item ranges over scoped threads and reassemble
//! the results in input order, so output bytes and the first reported error
//! are identical to a sequential pass. WebAssembly hosts run sequentially.

/// Items below this count, or per worker, are not worth a thread.
const MIN_ITEMS_PER_WORKER: usize = 8;
const MAX_WORKERS: usize = 16;

/// Applies `f` to every item and returns the results in input order.
pub(crate) fn map_ordered<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(usize, &T) -> R + Sync,
{
    let workers = worker_count(items.len());
    if workers <= 1 {
        return items
            .iter()
            .enumerate()
            .map(|(index, item)| f(index, item))
            .collect();
    }
    scoped_map(items, workers, &f)
}

#[cfg(not(target_family = "wasm"))]
fn worker_count(items: usize) -> usize {
    let available = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    available.min(MAX_WORKERS).min(items / MIN_ITEMS_PER_WORKER)
}

#[cfg(target_family = "wasm")]
fn worker_count(_items: usize) -> usize {
    1
}

#[cfg(not(target_family = "wasm"))]
fn scoped_map<T, R, F>(items: &[T], workers: usize, f: &F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(usize, &T) -> R + Sync,
{
    let span = items.len().div_ceil(workers);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(span)
            .enumerate()
            .map(|(part, chunk)| {
                scope.spawn(move || {
                    let base = part * span;
                    chunk
                        .iter()
                        .enumerate()
                        .map(|(offset, item)| f(base + offset, item))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut results = Vec::with_capacity(items.len());
        for handle in handles {
            match handle.join() {
                Ok(part) => results.extend(part),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        results
    })
}

#[cfg(target_family = "wasm")]
fn scoped_map<T, R, F>(items: &[T], _workers: usize, f: &F) -> Vec<R>
where
    F: Fn(usize, &T) -> R,
{
    items
        .iter()
        .enumerate()
        .map(|(index, item)| f(index, item))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::map_ordered;

    #[test]
    fn map_ordered_preserves_input_order_across_workers() {
        let items: Vec<u32> = (0..1000).collect();
        let mapped = map_ordered(&items, |index, item| (index, item * 3));
        assert!(
            mapped.iter().enumerate().all(
                |(position, &(index, value))| position == index && value == 3 * items[position]
            )
        );
    }
}
