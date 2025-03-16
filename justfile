repo := 'ghcr.io/josiahbull/send'

# Run formatting
fmt:
format:
    @cargo +nightly fmt

test:
    @cargo test --all-features --all-targets
    @cargo clippy
    @cargo mutants --colors=always --all-features --error true --no-shuffle --iterate -vV
    @cargo deny check
    @cargo semver-checks
    @cargo tree | grep openssl && exit 1 || exit 0

udeps:
    @cargo +nightly udeps

clean:
    @cargo clean
    @git clean -fdX

# Publish this crate to crates.io
publish:
    #TODO

default:
    @just --list
