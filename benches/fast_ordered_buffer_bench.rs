use std::time::Duration;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use fast_ordered_buffer::FobStreamExt;
use futures::stream::StreamExt;
use rand::{Rng, rng};
use tokio::runtime::Runtime;
use tokio::time::sleep;

/// Returns a stream of futures that, after a random delay (up to `max_delay_ms`), resolve to an item.
fn stream_of_random_delayed_futures<T: Clone + Send + 'static>(
    items: &[T],
    max_delay_ms: u64,
) -> impl futures::Stream<Item = impl futures::Future<Output = T> + Send> {
    let mut rng = rng();
    let iter = items.iter().cloned();
    futures::stream::iter(iter).map(move |x| {
        let delay = rng.random_range(0..=max_delay_ms);
        async move {
            sleep(Duration::from_millis(delay)).await;
            x
        }
    })
}

/// Benchmarks using the custom fast_ordered_buffer buffering.
async fn bench_fast_ordered_buffer(items: &[usize], concurrency: usize) {
    let stream = stream_of_random_delayed_futures(items, 5);
    let buffered = stream.fast_ordered_buffer(concurrency);
    let results: Vec<_> = buffered.collect().await;
    assert_eq!(results.len(), items.len());
    black_box(results);
}

/// Benchmarks using the futures buffered (ordered) buffering.
async fn bench_buffered(items: &[usize], concurrency: usize) {
    let stream = stream_of_random_delayed_futures(items, 5);
    let buffered = stream.buffered(concurrency);
    let results: Vec<_> = buffered.collect().await;
    assert_eq!(results.len(), items.len());
    black_box(results);
}

/// Benchmarks using the futures buffer_unordered buffering.
async fn bench_buffer_unordered(items: &[usize], concurrency: usize) {
    let stream = stream_of_random_delayed_futures(items, 5);
    let buffered = stream.buffer_unordered(concurrency);
    let results: Vec<_> = buffered.collect().await;
    assert_eq!(results.len(), items.len());
    black_box(results);
}

fn async_benchmarks(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Define two test cases: one with 1k items and another with 10k items.
    let small_items: Vec<usize> = (0..1_000).collect();
    let large_items: Vec<usize> = (0..10_000).collect();

    // --- Benchmarks for 1k items with concurrency 10 ---
    c.bench_function("fast_ordered_buffer 1k items concurrency=10", |b| {
        b.to_async(&rt)
            .iter(|| bench_fast_ordered_buffer(&small_items, 10))
    });
    c.bench_function("buffered 1k items concurrency=10", |b| {
        b.to_async(&rt).iter(|| bench_buffered(&small_items, 10))
    });
    c.bench_function("buffer_unordered 1k items concurrency=10", |b| {
        b.to_async(&rt)
            .iter(|| bench_buffer_unordered(&small_items, 10))
    });

    // --- Benchmarks for 10k items with concurrency 50 ---
    c.bench_function("fast_ordered_buffer 10k items concurrency=50", |b| {
        b.to_async(&rt)
            .iter(|| bench_fast_ordered_buffer(&large_items, 50))
    });
    c.bench_function("buffered 10k items concurrency=50", |b| {
        b.to_async(&rt).iter(|| bench_buffered(&large_items, 50))
    });
    c.bench_function("buffer_unordered 10k items concurrency=50", |b| {
        b.to_async(&rt)
            .iter(|| bench_buffer_unordered(&large_items, 50))
    });
}

criterion_group!(benches, async_benchmarks);
criterion_main!(benches);
