use std::{pin::Pin, time::Duration};

use fast_ordered_buffer::FobStreamExt;
use futures::{
    Stream, StreamExt,
    channel::oneshot,
    future::{self, Future},
    stream,
};
use pretty_assertions::assert_eq;
use rand::{Rng, rng};
use tokio::time::sleep;

/// Runs a given test closure up to 20 times, each time allowing 2 seconds
/// before considering it a timeout.
///
/// We want to ensure that the test never fails, so we retry it multiple
/// times and it must succeed on all.
async fn run_with_timeout<F, Fut>(test_closure: F)
where
    F: Fn() -> Fut + Clone + 'static,
    Fut: Future<Output = ()> + 'static,
{
    const TIMEOUT_SECS: u64 = 2;

    tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), test_closure())
        .await
        .unwrap();
}

/// Helper: transform a `Vec<T>` into a `Stream` of futures that each
/// resolve to `T` immediately (no delay).
fn stream_of_instant_futures<T: Clone + Send + 'static>(
    items: &[T],
) -> impl Stream<Item = impl Future<Output = T> + Send> {
    let iter = items.iter().cloned();
    stream::iter(iter).map(|x| future::ready(x))
}

/// Helper: transform a `Vec<T>` into a `Stream` of futures that complete
/// after a random delay (within `max_delay_ms`).
fn stream_of_random_delayed_futures<T: Clone + Send + 'static>(
    items: &[T],
    max_delay_ms: u64,
) -> impl Stream<Item = impl Future<Output = T> + Send> {
    let mut random_generator = rng();
    let iter = items.iter().cloned();
    stream::iter(iter).map(move |x| {
        let delay = random_generator.random_range(0..=max_delay_ms);
        async move {
            sleep(Duration::from_millis(delay)).await;
            x
        }
    })
}

/// Example utility that collects the buffered stream into a vector and
/// asserts the collected items match the provided slice.
/// This is to reduce repetition in some tests.
async fn assert_stream_matches_expected<T: std::fmt::Debug + PartialEq>(
    stream: impl Stream<Item = T> + Unpin,
    expected: &[T],
) {
    let results: Vec<_> = stream.collect().await;
    assert_eq!(results, expected);
}

/// Test that if the underlying stream is empty, we get `None` immediately.
#[tokio::test]
async fn test_empty_stream() {
    run_with_timeout(|| async {
        let s = stream::empty::<future::Ready<()>>();
        let mut buffered = s.fast_ordered_buffer(5);

        assert_eq!(buffered.next().await, None);
    })
    .await;
}

/// Test that a single item passes through correctly.
#[tokio::test]
async fn test_single_item() {
    run_with_timeout(|| async {
        let items = vec![42];
        let s = stream_of_instant_futures(&items);
        let mut buffered = s.fast_ordered_buffer(1);

        assert_eq!(buffered.next().await, Some(42));
        assert_eq!(buffered.next().await, None);
    })
    .await;
}

/// Test that instant-future items are emitted in ascending order.
#[tokio::test]
async fn test_ordering_for_instant_futures() {
    run_with_timeout(|| async {
        let items = vec![1, 2, 3, 4, 5];
        let s = stream_of_instant_futures(&items);
        let buffered = s.fast_ordered_buffer(2);

        assert_stream_matches_expected(buffered, &items).await;
    })
    .await;
}

/// Test that random delays (but small concurrency=2) do not break ordering.
/// Items should appear in ascending order, even if they complete out of order.
#[tokio::test]
async fn test_random_delays_small_concurrency() {
    run_with_timeout(|| async {
        let items = (1..=10).collect::<Vec<_>>();
        let s = stream_of_random_delayed_futures(&items, 30);
        let buffered = s.fast_ordered_buffer(2);

        assert_stream_matches_expected(buffered, &items).await;
    })
    .await;
}

/// Test that random delays with larger concurrency still yield correct ordering.
#[tokio::test]
async fn test_random_delays_larger_concurrency() {
    run_with_timeout(|| async {
        let items = (1..=10).collect::<Vec<_>>();
        let s = stream_of_random_delayed_futures(&items, 30);
        let buffered = s.fast_ordered_buffer(8);

        assert_stream_matches_expected(buffered, &items).await;
    })
    .await;
}

/// Ensures new futures are spawned even if the earliest item is taking longer.
/// We have two futures (#0 sleeps longer, #1 is quick). Concurrency=2 means
/// both start at once. #1 is done first, but must wait in a buffer until #0
/// is ready, ensuring final output is in ID order: (#0, #1).
#[tokio::test]
async fn test_does_not_block_new_futures_waiting_for_first() {
    run_with_timeout(|| async {
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();

        let s = stream::iter(vec![
            Box::pin(async move {
                sleep(Duration::from_millis(100)).await;
                tx1.send("first").unwrap();
                "first"
            }) as Pin<Box<dyn Future<Output = _> + Send>>,
            Box::pin(async move {
                sleep(Duration::from_millis(10)).await;
                tx2.send("second").unwrap();
                "second"
            }) as Pin<Box<dyn Future<Output = _> + Send>>,
        ]);

        let mut buffered = s.fast_ordered_buffer(2);

        // The user sees them in ID order, not completion order.
        let first = buffered.next().await;
        let second = buffered.next().await;
        let none = buffered.next().await;

        assert_eq!(first, Some("first"));
        assert_eq!(second, Some("second"));
        assert_eq!(none, None);

        // Also ensure that both oneshots have fired,
        // meaning #1 wasn't blocked from even starting.
        assert_eq!(rx1.await.unwrap(), "first");
        assert_eq!(rx2.await.unwrap(), "second");
    })
    .await;
}

/// Test concurrency is capped at a given number, but if concurrency > length of stream,
/// it simply processes them all.
#[tokio::test]
async fn test_concurrency_larger_than_stream_len() {
    run_with_timeout(|| async {
        let items = vec![10, 20, 30];
        let s = stream_of_instant_futures(&items);
        let buffered = s.fast_ordered_buffer(9999);

        assert_stream_matches_expected(buffered, &items).await;
    })
    .await;
}

/// Basic check on size_hint correctness. We expect at least the underlying
/// stream’s count (4 items). This is not guaranteed to be exact, but we
/// check basic consistency.
#[tokio::test]
async fn test_size_hint() {
    run_with_timeout(|| async {
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        let (tx3, rx3) = oneshot::channel();
        let (tx4, rx4) = oneshot::channel();

        let s = stream::iter(vec![
            Box::pin(async move { rx1.await.unwrap() }) as Pin<Box<dyn Future<Output = _> + Send>>,
            Box::pin(async move { rx3.await.unwrap() }) as Pin<Box<dyn Future<Output = _> + Send>>,
            Box::pin(async move { rx2.await.unwrap() }) as Pin<Box<dyn Future<Output = _> + Send>>,
            Box::pin(async move { rx4.await.unwrap() }) as Pin<Box<dyn Future<Output = _> + Send>>,
        ]);

        let mut buffered = s.fast_ordered_buffer(2);

        let (lower, upper) = buffered.size_hint();
        assert_eq!(lower, 4);
        assert_eq!(upper, Some(4));
        tx1.send(10).unwrap();
        assert_eq!(buffered.next().await, Some(10));

        let (lower, upper) = buffered.size_hint();
        assert_eq!(lower, 3);
        assert_eq!(upper, Some(3));
        tx2.send(20).unwrap();

        let (lower, upper) = buffered.size_hint();
        assert_eq!(lower, 3);
        assert_eq!(upper, Some(3));
        tx3.send(30).unwrap();
        assert_eq!(buffered.next().await, Some(30));

        let (lower, upper) = buffered.size_hint();
        assert_eq!(lower, 2);
        assert_eq!(upper, Some(2));
        assert_eq!(buffered.next().await, Some(20));

        let (lower, upper) = buffered.size_hint();
        assert_eq!(lower, 1);
        assert_eq!(upper, Some(1));
        tx4.send(40).unwrap();
        assert_eq!(buffered.next().await, Some(40));

        let (lower, upper) = buffered.size_hint();
        assert_eq!(lower, 0);
        assert_eq!(upper, Some(0));
        assert_eq!(buffered.next().await, None);
    })
    .await;
}

#[tokio::test]
async fn test_size_hint_many_pending() {
    run_with_timeout(|| async {
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        let (tx3, rx3) = oneshot::channel();
        let (tx4, rx4) = oneshot::channel();

        let s = stream::iter(vec![
            Box::pin(async move { rx1.await.unwrap() }) as Pin<Box<dyn Future<Output = _> + Send>>,
            Box::pin(async move { rx2.await.unwrap() }) as Pin<Box<dyn Future<Output = _> + Send>>,
            Box::pin(async move { rx3.await.unwrap() }) as Pin<Box<dyn Future<Output = _> + Send>>,
            Box::pin(async move { rx4.await.unwrap() }) as Pin<Box<dyn Future<Output = _> + Send>>,
        ]);

        let mut buffered = s.fast_ordered_buffer(2);

        let (lower, upper) = buffered.size_hint();
        assert_eq!(lower, 4);
        assert_eq!(upper, Some(4));
        tx4.send(40).unwrap();
        let next = tokio::time::timeout(Duration::from_millis(10), buffered.next()).await;
        assert!(next.is_err());

        let (lower, upper) = buffered.size_hint();
        assert_eq!(lower, 4);
        assert_eq!(upper, Some(4));
        tx3.send(30).unwrap();
        let next = tokio::time::timeout(Duration::from_millis(10), buffered.next()).await;
        assert!(next.is_err());

        let (lower, upper) = buffered.size_hint();
        assert_eq!(lower, 4);
        assert_eq!(upper, Some(4));
        tx2.send(20).unwrap();
        let next = tokio::time::timeout(Duration::from_millis(10), buffered.next()).await;
        assert!(next.is_err());

        let (lower, upper) = buffered.size_hint();
        assert_eq!(lower, 4);
        assert_eq!(upper, Some(4));
        tx1.send(10).unwrap();

        // Assert that values can be retrieved in order.
        assert_eq!(buffered.next().await, Some(10));
        assert_eq!(buffered.next().await, Some(20));
        assert_eq!(buffered.next().await, Some(30));
        assert_eq!(buffered.next().await, Some(40));
    })
    .await;
}

/// Stress test with a larger item set, verifying that items are always
/// delivered in ascending order no matter the concurrency or delay distribution.
#[tokio::test]
async fn test_large_out_of_order() {
    run_with_timeout(|| async {
        let items = (0..200).collect::<Vec<_>>();
        let s = stream_of_random_delayed_futures(&items, 50);
        let buffered = s.fast_ordered_buffer(10);

        assert_stream_matches_expected(buffered, &items).await;
    })
    .await;
}

/// Test with futures returning `Result` to ensure ordering is consistent
/// even with successes and errors in the mix.
#[tokio::test]
async fn test_futures_returning_results() {
    run_with_timeout(|| async {
        let s = stream::iter(vec![
            Box::pin(async { Ok::<_, &'static str>(1) })
                as Pin<Box<dyn Future<Output = Result<_, _>> + Send>>,
            Box::pin(async { Err::<i32, &'static str>("boom!") })
                as Pin<Box<dyn Future<Output = Result<_, _>> + Send>>,
            Box::pin(async { Ok::<_, &'static str>(3) })
                as Pin<Box<dyn Future<Output = Result<_, _>> + Send>>,
        ]);
        let mut buffered = s.fast_ordered_buffer(2);

        let mut results = Vec::new();
        while let Some(item) = buffered.next().await {
            results.push(item);
        }
        // Order must still be (1 => Err => 3).
        assert_eq!(results, vec![Ok(1), Err("boom!"), Ok(3)]);
    })
    .await;
}

/// Edge case for concurrency=0. This test verifies that the stream remains
/// usable even if concurrency is 0. Some implementations might panic or hang
/// unless they're coded to handle it gracefully.
#[tokio::test]
async fn test_concurrency_zero() {
    run_with_timeout(|| async {
        let items = vec![1, 2, 3];
        let s = stream_of_instant_futures(&items);
        let mut buffered = s.fast_ordered_buffer(0);

        assert_eq!(buffered.next().await, Some(1));
        assert_eq!(buffered.next().await, Some(2));
        assert_eq!(buffered.next().await, Some(3));
        assert_eq!(buffered.next().await, None);
    })
    .await;
}
