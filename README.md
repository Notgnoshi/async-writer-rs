# async-writer-rs

A Rust Write wrapper that defers writing to a background thread. There are some use-cases on certain
hardware where even a `BufWriter` blocks too long.

# How to use

```rust
let writer_to_wrap = Vec::new();
let mut writer = AsyncWriter::new(writer_to_wrap);
writer.write_all(&[1, 2, 3, 4]).unwrap();
writer.flush().unwrap();

let written_bytes = writer.take_writer().unwrap();
assert_eq!(written_bytes, [1, 2, 3, 4]);
```

# TODO

* [ ] Better error handling in case of failed writes; don't panic, return errors
* [ ] Benchmark `AsyncWriter` against `BufWriter` on IO limited hardware
