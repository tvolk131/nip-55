use std::io::{self, Read};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;

pub struct SyncReader {
    task_handle: tokio::task::JoinHandle<()>,
    receiver: mpsc::Receiver<u8>,
}

impl SyncReader {
    pub fn new<R: AsyncRead + Unpin + Send + 'static>(mut async_reader: R) -> Self {
        let (sender, receiver) = mpsc::channel(1024);

        let task_handle = tokio::task::spawn(async move {
            let mut buffer = [0; 1024];
            loop {
                match async_reader.read(&mut buffer).await {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        for &byte in &buffer[..n] {
                            if sender.send(byte).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => break, // Handle error
                }
            }
        });

        Self {
            task_handle,
            receiver,
        }
    }
}

impl Read for SyncReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut count = 0;
        for byte in buf.iter_mut() {
            match self.receiver.blocking_recv() {
                Some(b) => {
                    *byte = b;
                    count += 1;
                }
                None => {
                    self.task_handle.abort();
                    break;
                }
            }
        }
        Ok(count)
    }
}
