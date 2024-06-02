use crate::uds::UdsMessage;
use futures::channel::mpsc;
use serde_json::{Deserializer, StreamDeserializer};
use std::path::Path;
use tokio::net::{unix::OwnedReadHalf, UnixListener};
use tokio_util::io::StreamReader;

use super::reader::SyncReader;

pub struct UnixDomainSocketServerTransport<IncomingMessage: UdsMessage, OutgoingMessage: UdsMessage>
{
    uds_task_handle: tokio::task::JoinHandle<()>,
    uds_address: String,
    phantom: std::marker::PhantomData<(IncomingMessage, OutgoingMessage)>,
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
    pub fn connect_and_start(
        uds_address: impl Into<String>,
        mut handler: impl FnMut(mpsc::Receiver<IncomingMessage>, mpsc::Sender<OutgoingMessage>)
            + Send
            + 'static,
    ) -> std::io::Result<Self> {
        let uds_address = uds_address.into();

        if Path::new(&uds_address).exists() {
            std::fs::remove_file(&uds_address)?;
        }

        let listener = UnixListener::bind(&uds_address)?;

        let uds_task_handle = tokio::spawn(async move {
            loop {
                if let Ok((socket, _)) = listener.accept().await {
                    let (mut incoming_message_tx, incoming_message_rx) = mpsc::channel(1024);
                    let (outgoing_message_tx, mut outgoing_message_rx) = mpsc::channel(1024);

                    let (socket_reader, socket_writer) = socket.into_split();

                    // TODO: Grab the task handle and cancel it when the server is dropped.
                    tokio::task::spawn(async move {
                        let mut incoming_message_stream =
                            Deserializer::from_reader(SyncReader::new(socket_reader))
                                .into_iter::<IncomingMessage>();

                        loop {
                            if let Some(incoming_message_or) = incoming_message_stream.next() {
                                match incoming_message_or {
                                    Ok(incoming_message) => {
                                        incoming_message_tx.try_send(incoming_message).unwrap();
                                    }
                                    Err(e) => {
                                        eprintln!("Error deserializing incoming message: {:?}", e);
                                    }
                                }
                            }
                        }
                    });

                    // TODO: Grab the task handle and cancel it when the server is dropped.
                    tokio::spawn(async move {
                        loop {
                            // Write from `outgoing_message_rx` to `socket`.
                            while let Ok(Some(outgoing_message)) = outgoing_message_rx.try_next() {
                                let serialized_outgoing_message =
                                    serde_json::to_vec(&outgoing_message).unwrap();
                                println!(
                                    "serialized_outgoing_message: {:?}",
                                    serialized_outgoing_message
                                );
                                socket_writer
                                    .try_write(&serialized_outgoing_message)
                                    .unwrap();
                            }

                            tokio::task::yield_now().await;
                        }
                    });

                    handler(incoming_message_rx, outgoing_message_tx);
                }
            }
        });

        Ok(Self {
            uds_task_handle,
            uds_address,
            phantom: std::marker::PhantomData,
        })
    }
}
