use crate::uds::UdsMessage;
use futures::SinkExt;
use serde_json::StreamDeserializer;
use std::path::Path;
use std::pin::Pin;
use tokio::net::UnixListener;
use tokio_util::io::StreamReader;

pub struct UnixDomainSocketServerTransport<IncomingMessage: UdsMessage, OutgoingMessage: UdsMessage>
{
    uds_task_handle: tokio::task::JoinHandle<()>,
    rpc_receiver: futures::channel::mpsc::Receiver<(
        futures::channel::mpsc::Receiver<IncomingMessage>,
        futures::channel::mpsc::Sender<OutgoingMessage>,
    )>,
    uds_address: String,
}

impl<IncomingMessage: UdsMessage, OutgoingMessage: UdsMessage> std::ops::Drop
    for UnixDomainSocketServerTransport<IncomingMessage, OutgoingMessage>
{
    fn drop(&mut self) {
        // Abort the UDS task, since it will loop forever otherwise.
        self.uds_task_handle.abort();

        // Try to remove the UDS file. If it fails, it's not a big deal.
        let _ = std::fs::remove_file(&self.uds_address);
    }
}

impl<IncomingMessage: UdsMessage, OutgoingMessage: UdsMessage>
    UnixDomainSocketServerTransport<IncomingMessage, OutgoingMessage>
{
    /// Create a new `UnixDomainSocketServerTransport` and start listening for incoming
    /// connections. **MUST** be called from within a tokio runtime.
    pub fn connect_and_start(uds_address: impl Into<String>) -> std::io::Result<Self> {
        let uds_address = uds_address.into();

        if Path::new(&uds_address).exists() {
            std::fs::remove_file(&uds_address)?;
        }

        // Queue for incoming requests to the server.
        let (rpc_sender, rpc_receiver) = futures::channel::mpsc::channel(1024);

        let listener = UnixListener::bind(&uds_address)?;

        let uds_task_handle = tokio::spawn(async move {
            loop {
                let mut rpc_sender_clone = rpc_sender.clone();

                if let Ok((socket, _)) = listener.accept().await {
                    // TODO: Grab the task handle and cancel it when the server is dropped.
                    tokio::spawn(async move {
                        let (mut incoming_message_tx, incoming_message_rx) =
                            futures::channel::mpsc::channel(1024);
                        let (outgoing_message_tx, mut outgoing_message_rx) =
                            futures::channel::mpsc::channel(1024);

                        // Per-connection task.
                        // TODO: Grab the task handle and cancel it when the server is dropped.
                        tokio::task::spawn_blocking(move || {
                            let (mut incoming_byte_stream_tx, incoming_byte_stream_rx) =
                                futures::channel::mpsc::channel::<
                                    Result<tokio_util::bytes::Bytes, std::io::Error>,
                                >(1024);

                            let mut incoming_message_stream = StreamDeserializer::new(
                                serde_json::de::IoRead::new(tokio_util::io::SyncIoBridge::new(
                                    StreamReader::new(incoming_byte_stream_rx),
                                )),
                            );

                            loop {
                                // panic!("Top of loop!");

                                // Write from `socket` to `incoming_byte_stream_tx` which seeds `incoming_message_stream`.
                                let mut buf = vec![0; 1024];
                                match socket.try_read(&mut buf) {
                                    Ok(0) => {
                                        continue;
                                    }
                                    Ok(n) => {
                                        incoming_byte_stream_tx
                                            .try_send(Ok(
                                                tokio_util::bytes::Bytes::copy_from_slice(
                                                    &buf[..n],
                                                ),
                                            ))
                                            .unwrap();
                                    }
                                    Err(e) => {
                                        if e.kind() == std::io::ErrorKind::WouldBlock {
                                            continue;
                                        } else {
                                            incoming_byte_stream_tx.try_send(Err(e)).unwrap();
                                            break;
                                        }
                                    }
                                }

                                // Now that we've seeded `incoming_message_stream`, we can read from it.
                                if let Some(incoming_message_or) = incoming_message_stream.next() {
                                    if let Ok(incoming_message) = incoming_message_or {
                                        incoming_message_tx.try_send(incoming_message).unwrap();
                                    }
                                } else {
                                    break;
                                }

                                // Write from `outgoing_message_rx` to `socket`.
                                while let Ok(Some(outgoing_message)) =
                                    outgoing_message_rx.try_next()
                                {
                                    let serialized_outgoing_message =
                                        serde_json::to_vec(&outgoing_message).unwrap();
                                    println!(
                                        "serialized_outgoing_message: {:?}",
                                        serialized_outgoing_message
                                    );
                                    socket.try_write(&serialized_outgoing_message).unwrap();
                                }
                            }
                        });

                        // TODO: Remove this unwrap. For now it's safe because the receiver will only be dropped when the server is dropped.
                        rpc_sender_clone
                            .send((incoming_message_rx, outgoing_message_tx))
                            .await
                            .unwrap();
                    });
                }
            }
        });

        Ok(Self {
            uds_task_handle,
            rpc_receiver,
            uds_address,
        })
    }

    fn project(
        self: Pin<&mut Self>,
    ) -> Pin<
        &mut futures::channel::mpsc::Receiver<(
            futures::channel::mpsc::Receiver<IncomingMessage>,
            futures::channel::mpsc::Sender<OutgoingMessage>,
        )>,
    > {
        unsafe { self.map_unchecked_mut(|x| &mut x.rpc_receiver) }
    }
}

impl<IncomingMessage: UdsMessage, OutgoingMessage: UdsMessage> futures::Stream
    for UnixDomainSocketServerTransport<IncomingMessage, OutgoingMessage>
{
    type Item = (
        futures::channel::mpsc::Receiver<IncomingMessage>,
        futures::channel::mpsc::Sender<OutgoingMessage>,
    );

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.project().poll_next(cx)
    }
}
