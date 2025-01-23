use std::io::Write;
use std::sync::mpsc::{sync_channel, Receiver, SendError, SyncSender, TrySendError};

struct BackgroundWorker<W: Write> {
    writer: W,
    receiver: Receiver<Command>,
}

impl<W: Write> BackgroundWorker<W> {
    fn new(writer: W, receiver: Receiver<Command>) -> Self {
        Self { writer, receiver }
    }

    fn spin(&mut self) {
        while let Ok(record) = self.receiver.recv() {
            match record {
                Command::WriteBlob(blob) => self.write_blob(blob),
                Command::Flush => self.flush(),
                Command::Terminate => break,
            }
        }
    }

    fn write_blob(&mut self, blob: Vec<u8>) {
        // This panic will get propogated as a disconnected channel, which will cause the next
        // AsyncWriter::write() to fail, and AsyncWriter::drop() to panic
        self.writer
            .write_all(&blob)
            .expect("Background write failed");
    }

    fn flush(&mut self) {
        self.writer.flush().expect("Background flush failed");
    }

    fn take_writer(self) -> W {
        self.writer
    }
}

enum Command {
    WriteBlob(Vec<u8>),
    Flush,
    Terminate,
}

/// A [Write] implementation that defers to a background thread
///
/// On modern desktop hardware, this might not be necessary, but on hardware with slower disks, or
/// from code that's timing sensitive, it is often not enough of a performance improvement to use
/// [std::io::BufWriter], and it's necessary to defer to a background thread.
///
/// # Failed writes
///
/// If the background thread fails to [Write::write] or [Write::flush], it will panic, which will
/// be propagated back up to the calling thread on the next `write` or `flush` call. This means
/// that you cannot detect exactly which bytes were successfully written, and which bytes were left
/// in-flight.
pub struct AsyncWriter<W: Write + Send + 'static> {
    sender: SyncSender<Command>,
    handle: Option<std::thread::JoinHandle<W>>,
}

impl<W: Write + Send + 'static> Drop for AsyncWriter<W> {
    fn drop(&mut self) {
        if self.handle.is_some() {
            let _writer = self.terminate();
        }
    }
}

impl<W: Write + Send + 'static> AsyncWriter<W> {
    fn terminate(&mut self) -> std::io::Result<W> {
        // Send the terminate signal
        while let Err(TrySendError::Full(_)) = self.sender.try_send(Command::Terminate) {
            // std::thread::yield_now();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        // Join the background thread
        let handle = self
            .handle
            .take()
            .expect("Background thread was already joined");
        handle.join().map_err(|_e| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "Background thread panicked while writing",
            )
        })
    }

    /// Create a new [AsyncWriter] using default parameters
    ///
    /// Use [AsyncWriterBuilder] to customize the writer parameters
    pub fn new(writer: W) -> Self {
        AsyncWriterBuilder::default().build(writer)
    }

    /// Flush any in-flight writes, and return the underlying writer
    pub fn take_writer(mut self) -> std::io::Result<W> {
        self.terminate()
    }
}

impl<W: Write + Send + 'static> Write for AsyncWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // We have to copy the buffer in order to write it from a background thread.
        let blob = Command::WriteBlob(buf.to_vec());
        match self.sender.send(blob) {
            Ok(_) => Ok(buf.len()),
            Err(SendError(_failed_buf)) => Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "Background thread terminated",
            )),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let command = Command::Flush;
        match self.sender.send(command) {
            Ok(_) => Ok(()),
            Err(SendError(_)) => Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "Background thread terminated",
            )),
        }
    }
}

pub struct AsyncWriterBuilder {
    thread_name: String,
    /// Number of blobs allowed to queue up.
    ///
    /// Every [Writer::write] call corresponds to a single blob. The use of a
    /// [BufWriter](std::io::BufWriter) to cut down on the number of allocations and memcpys is
    /// recommended
    queue_size: usize,
}

impl Default for AsyncWriterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncWriterBuilder {
    pub fn new() -> Self {
        Self {
            thread_name: String::from("async-writer"),
            queue_size: 1024,
        }
    }

    /// Set the thread name for the [AsyncWriter]s background worker thread
    pub fn with_thread_name<S: Into<String>>(mut self, name: S) -> Self {
        self.thread_name = name.into();
        self
    }

    /// Allow the given number of in-flight writes
    ///
    /// Each item queued is a single [AsyncWriter::write] call. If not set, the default value is
    /// 1024
    pub fn with_queue_size(mut self, len: usize) -> Self {
        self.queue_size = len;
        self
    }

    pub fn build<W: Write + Send + 'static>(self, writer: W) -> AsyncWriter<W> {
        let (sender, receiver) = sync_channel(self.queue_size);
        let handle = std::thread::Builder::new()
            .name(self.thread_name)
            .spawn(move || {
                let mut worker = BackgroundWorker::new(writer, receiver);
                worker.spin();
                worker.take_writer()
            })
            .expect("Failed to spawn background worker thread");
        let handle = Some(handle);
        AsyncWriter { sender, handle }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple() {
        let dest = Vec::new();
        let mut async_writer = AsyncWriterBuilder::default()
            .with_thread_name("test-writer")
            .with_queue_size(10)
            .build(dest);
        async_writer.write_all(&[1, 2, 3]).unwrap();
        let writer = async_writer.take_writer().unwrap();
        assert_eq!(writer, [1, 2, 3]);
    }

    #[test]
    fn test_failed_write() {
        let dest: [u8; 4] = [0, 0, 0, 0];
        let dest = std::io::Cursor::new(dest);
        let mut writer = AsyncWriter::new(dest);
        writer.write_all(&[1, 1, 1, 1]).unwrap();
        let result = writer.write_all(&[2]); // This write fails, because the writer only has room for 4 bytes
        assert!(result.is_ok());

        let writer = writer.take_writer();
        assert!(writer.is_err());
    }
}
