# Fast Ordered Buffer

[![CI](https://github.com/JosiahBull/fast-ordered-buffer/actions/workflows/ci.yaml/badge.svg)](https://github.com/JosiahBull/fast-ordered-buffer/actions/workflows/ci.yaml)
[![Crates.io](https://img.shields.io/crates/v/fast_ordered_buffer)](https://crates.io/crates/fast_ordered_buffer)
[![Docs.rs](https://docs.rs/human-friendly-ids/badge.svg)](https://docs.rs/fast_ordered_buffer)

This Rust library is a faster alternative to [buffered](https://docs.rs/futures/latest/futures/stream/trait.StreamExt.html#method.buffered) from the [futures](https://docs.rs/futures) crate. It is used for buffering a stream of items into a fixed-size buffer, while preserving the order of the items.

The futures library is a great library, but it is desgined to use zero or minimal cost abstractions - even if this results in worse performance. In the case of `.buffered` it will wait for a slow future to complete to retain ordering - even if there are more futures waiting to be spawned concurrently.

This library is designed to allocate those waiting futures into a priority queue, and retain ordering while allowing futures to complete as fast as possible. This comes at the downside of a small amount of memory overhead (perfect case - 0 overhead, but we do not bound the number of futures that may be saved).

## Recommendations for usage

1. This crate does not provide any form of backpressure, and so might be unsuitable for use in a high-throughput environment. If you need backpressure, consider using the [futures](https://docs.rs/futures) crate, or submit an issue here and it can be added as a feature.
2. Ensure the futures contained within this crate have a timeout.

## Getting Started

```toml
[dependencies]
fast_ordered_buffer = "0.1.0"
```

### Usage

```rust
use fast_ordered_buffer::FobStreamExt;
use futures::stream::{StreamExt, FuturesUnordered};
use futures::{future::FutureExt, stream};

async fn wait_then_return(i: u32, time: std::time::Duration) -> u32 {
    tokio::time::sleep(time).await;
    i
}

#[tokio::main]
async fn main() {
    let futures = vec![
        wait_then_return(5, std::time::Duration::from_millis(500)),
        wait_then_return(1, std::time::Duration::from_millis(100)),
        wait_then_return(2, std::time::Duration::from_millis(200)),
        wait_then_return(3, std::time::Duration::from_millis(300)),
        wait_then_return(4, std::time::Duration::from_millis(400)),
    ];

    // Create a stream of futures.
    let now = std::time::Instant::now();
    let result = stream::iter(futures)
        // Buffer the futures into a fixed-size buffer.
        // Try changing this to `buffered` to see the difference in performance.
        .fast_ordered_buffer(2)
        // Collect the results of the futures into a vector.
        .collect::<Vec<_>>()
        .await;

    println!("Result: {:?}, Time taken: {:?}", result, now.elapsed());
}
```

## Performance

### Methodology

All benchmarks taken on a Macbook Pro M2, standard YMMV disclaimer applies.

Tests

- 10k Random Futures (Concurrency = 50)
- 1k Random Futures (Concurrency = 10)

We use criterion to benchmark the performance of this library against the futures library. The benchmark is run with the following command:

```bash
cargo bench
```

We take the Mean time taken for each buffer type, `fast_ordered_buffer`, `unordered_buffer`, and `buffer`, rounded to the nearest millisecond.

### Results

| Buffer Type         | 10k Random Futures (Concurrency = 50) | 1k Random Futures (Concurrency = 10) |
|---------------------|---------------------------------------|--------------------------------------|
| `fast_ordered_buffer` | 755ms                                 | 399ms                                |
| `unordered_buffer`    | 734ms                                 | 366ms                                |
| `buffer`              | 1169ms                                | 514ms                                |

## Contribution

Please feel free to submit a pull request or open an issue if you have any suggestions or improvements.

## License

This project is licensed under either of

- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)

at your option.
