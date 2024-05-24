use super::super::{UdsClientError, UdsMessage};
use futures::Stream;
use serde_json::de::IoRead;
use tokio::net::UnixStream;
use tokio::{io::AsyncWriteExt, net::unix::OwnedWriteHalf};
use tokio_util::io::SyncIoBridge;

pub struct UnixDomainSocketClientTransport<OutgoingMessage: UdsMessage, IncomingMessage: UdsMessage>
{
    uds_address: String,
    socket_writer: OwnedWriteHalf,
    incoming_message_rx: std::sync::mpsc::Receiver<IncomingMessage>,
    phantom: std::marker::PhantomData<OutgoingMessage>,
}

impl<OutgoingMessage: UdsMessage, IncomingMessage: UdsMessage>
    UnixDomainSocketClientTransport<OutgoingMessage, IncomingMessage>
{
    pub async fn new(uds_address: impl Into<String>) -> Result<Self, UdsClientError> {
        let uds_address = uds_address.into();

        // Open up a UDS connection to the server.
        let socket = UnixStream::connect(&uds_address)
            .await
            .map_err(|_| UdsClientError::ServerNotRunning)?;

        let (socket_reader, socket_writer) = socket.into_split();

        let (incoming_message_tx, incoming_message_rx) = std::sync::mpsc::channel();

        tokio::task::spawn_blocking(move || {
            let mut stream_deserializer =
                serde_json::StreamDeserializer::new(IoRead::new(SyncIoBridge::new(socket_reader)));

            while let Some(Ok(incoming_message)) = stream_deserializer.next() {
                incoming_message_tx.send(incoming_message).unwrap();
            }
        });

        Ok(Self {
            uds_address,
            socket_writer,
            incoming_message_rx,
            phantom: std::marker::PhantomData,
        })
    }

    pub async fn send_outgoing_message(
        &mut self,
        outgoing_message: OutgoingMessage,
    ) -> Result<(), UdsClientError> {
        let msg = serde_json::to_vec(&outgoing_message)
            .map_err(|_| UdsClientError::RequestSerializationError)?;

        self.socket_writer
            .write_all(&msg)
            .await
            .map_err(|_| UdsClientError::UdsSocketError)
    }
}

impl<
        OutgoingMessage: UdsMessage + std::marker::Unpin,
        IncomingMessage: UdsMessage + std::marker::Unpin,
    > Stream for UnixDomainSocketClientTransport<OutgoingMessage, IncomingMessage>
{
    type Item = IncomingMessage;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.incoming_message_rx.try_recv() {
            Ok(incoming_message) => std::task::Poll::Ready(Some(incoming_message)),
            Err(std::sync::mpsc::TryRecvError::Empty) => std::task::Poll::Pending,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => std::task::Poll::Ready(None),
        }
    }
}
