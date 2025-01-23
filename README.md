# async-writer-rs
A Rust Write wrapper that defers writing to a background thread. There are some use-cases on certain
hardware where even a `BufWriter` blocks too long.

# TODO

* [ ] Better error handling in case of failed writes; don't panic, return errors
* [ ] Benchmark `AsyncWriter` against `BufWriter` on IO limited hardware
