#![doc = include_str!("../README.md")]
#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::uninlined_format_args)]

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::num::NonZeroUsize;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures::Sink;
use futures::stream::{Fuse, FuturesUnordered, Stream, StreamExt};
use pin_project_lite::pin_project;

/// A wrapper struct for heap entries so the smallest ID is considered the "largest" item in the
/// [`BinaryHeap`] (and thus popped first).
struct Pending<O> {
    id: usize,
    output: O,
}

#[cfg_attr(test, mutants::skip)]
impl<O> PartialEq for Pending<O> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<O> Eq for Pending<O> {}

impl<O> Ord for Pending<O> {
    fn cmp(&self, other: &Self) -> Ordering {
        // We flip the comparison so that lower IDs have higher priority.
        other.id.cmp(&self.id)
    }
}

impl<O> PartialOrd for Pending<O> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pin_project! {
    pub struct IdentifiableFuture<Fut> {
        id: usize,
        #[pin]
        fut: Fut,
    }
}

impl<F> Future for IdentifiableFuture<F>
where
    F: Future,
{
    type Output = (usize, F::Output);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        this.fut.poll(cx).map(|x| (*this.id, x))
    }
}

pin_project! {
    pub struct FastBufferOrdered<St>
    where
        St: Stream,
        St::Item: Future,
    {
        #[pin]
        stream: Fuse<St>,
        in_progress_queue: FuturesUnordered<IdentifiableFuture<St::Item>>,
        max: Option<NonZeroUsize>,
        next_id: usize,
        pending_release: BinaryHeap<Pending<<St::Item as Future>::Output>>,
        waiting_for: usize,
    }
}

impl<St> FastBufferOrdered<St>
where
    St: Stream,
    St::Item: Future,
{
    pub fn new(stream: St, n: Option<usize>) -> Self {
        Self {
            stream: stream.fuse(),
            in_progress_queue: FuturesUnordered::new(),
            max: n.and_then(NonZeroUsize::new),
            next_id: 0,
            pending_release: BinaryHeap::new(),
            waiting_for: 0,
        }
    }
}

impl<St> Stream for FastBufferOrdered<St>
where
    St: Stream,
    St::Item: Future,
{
    type Item = <St::Item as Future>::Output;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        // First up, try to spawn off as many futures as possible by filling up
        // our queue of futures.
        while this
            .max
            .map(|max| this.in_progress_queue.len() < max.get())
            .unwrap_or(true)
        {
            match this.stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(fut)) => {
                    let fut = IdentifiableFuture {
                        id: *this.next_id,
                        fut,
                    };
                    this.in_progress_queue.push(fut);
                    *this.next_id += 1;
                }
                Poll::Ready(None) | Poll::Pending => break,
            }
        }

        // Attempt to pull the next value from the in_progress_queue
        while let Poll::Ready(Some((id, output))) = this.in_progress_queue.poll_next_unpin(cx) {
            if id == *this.waiting_for {
                *this.waiting_for += 1;
                return Poll::Ready(Some(output));
            }
            this.pending_release.push(Pending { id, output });
        }

        if let Some(next) = this.pending_release.peek() {
            if next.id == *this.waiting_for {
                *this.waiting_for += 1;
                return Poll::Ready(Some(this.pending_release.pop().unwrap().output));
            }
        }

        // If more values are still coming from the stream, we're not done yet
        if this.stream.is_done() && this.in_progress_queue.is_empty() {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let queue_len = self.in_progress_queue.len() + self.pending_release.len();
        let (lower, upper) = self.stream.size_hint();
        let lower = lower.saturating_add(queue_len);
        let upper = match upper {
            Some(x) => x.checked_add(queue_len),
            None => None,
        };
        (lower, upper)
    }
}

pub trait FobStreamExt: Stream {
    fn fast_ordered_buffer(self, n: usize) -> FastBufferOrdered<Self>
    where
        Self: Sized,
        Self::Item: Future,
    {
        FastBufferOrdered::new(self, Some(n))
    }
}
impl<T: Stream> FobStreamExt for T {}

// Forwarding impl of Sink from the underlying stream
#[cfg_attr(test, mutants::skip)]
impl<S, Item> Sink<Item> for FastBufferOrdered<S>
where
    S: Stream + Sink<Item>,
    S::Item: Future,
{
    type Error = S::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().stream.poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: Item) -> Result<(), Self::Error> {
        self.project().stream.start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().stream.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().stream.poll_close(cx)
    }
}
