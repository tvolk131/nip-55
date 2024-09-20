use super::nip04_jsonrpc::{
    jsonrpc_response_to_nip04_encrypted_event, nip04_encrypted_event_to_jsonrpc_request,
};
use crate::json_rpc::{JsonRpcResponseData, JsonRpcServerStream, SingleOrBatch};
use crate::{
    json_rpc::{JsonRpcRequest, JsonRpcResponse, JsonRpcServerTransport},
    uds_req_res::server::UnixDomainSocketServerTransport,
};
use futures::{FutureExt, StreamExt};
use nostr_sdk::{Event, Keys, PublicKey, SecretKey};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

pub trait KeyManager: Send + Sync {
    // TODO: Make this async.
    fn get_secret_key(&self, public_key: &PublicKey) -> Option<SecretKey>;
}

pub struct Nip55ServerStream {
    #[allow(clippy::type_complexity)]
    stream: Pin<
        Box<
            dyn futures::Stream<
                    Item = (
                        SingleOrBatch<JsonRpcRequest>,
                        SecretKey,
                        futures::channel::oneshot::Sender<SingleOrBatch<JsonRpcResponseData>>,
                    ),
                > + Send,
        >,
    >,
}

impl futures::Stream for Nip55ServerStream {
    type Item = (
        SingleOrBatch<JsonRpcRequest>,
        SecretKey,
        futures::channel::oneshot::Sender<SingleOrBatch<JsonRpcResponseData>>,
    );

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.stream.poll_next_unpin(cx)
    }
}

impl Nip55ServerStream {
    pub fn start<SingleOrBatchRequest: AsRef<SingleOrBatch<JsonRpcRequest>> + Send + 'static>(
        uds_address: impl Into<String>,
        key_manager: Arc<dyn KeyManager>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            stream: Box::pin(
                JsonRpcServerStream::start(Nip55ServerTransport::connect_and_start(
                    uds_address,
                    key_manager,
                )?)
                .map(|((req, secret_key), res_sender)| (req, secret_key, res_sender)),
            ),
        })
    }
}

struct Nip55ServerTransport {
    transport_server: UnixDomainSocketServerTransport<Event, Event>,
    key_manager: Arc<dyn KeyManager>,
}

impl Nip55ServerTransport {
    /// Create a new `Nip55ServerTransport` and start listening for incoming
    /// connections. **MUST** be called from within a tokio runtime.
    fn connect_and_start(
        uds_address: impl Into<String>,
        key_manager: Arc<dyn KeyManager>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            transport_server: UnixDomainSocketServerTransport::connect_and_start(uds_address)?,
            key_manager,
        })
    }
}

impl futures::Stream for Nip55ServerTransport {
    type Item = (
        (SingleOrBatch<JsonRpcRequest>, SecretKey),
        futures::channel::oneshot::Sender<SingleOrBatch<JsonRpcResponse>>,
    );

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let (request_event, response_event_sender) = match self.transport_server.poll_next_unpin(cx)
        {
            Poll::Ready(Some((request_event, response_event_sender))) => {
                (request_event, response_event_sender)
            }
            Poll::Ready(None) | Poll::Pending => return Poll::Pending,
        };

        let request_event_kind = request_event.kind;
        let request_event_author = request_event.pubkey;

        // TODO: Should we attempt to NIP-04 decrypt the request for all public keys rather than just the first one?
        let Some(user_public_key) = request_event.public_keys().next() else {
            // TODO: Should we send a response to `response_event_sender`? What secret key should we use to sign it?
            return self.poll_next(cx);
        };

        let Some(user_secret_key) = self.key_manager.get_secret_key(user_public_key) else {
            // TODO: Should we send a response to `response_event_sender`? What secret key should we use to sign it?
            return self.poll_next(cx);
        };

        let user_keypair = Keys::new(user_secret_key.clone());

        let Ok(request) = nip04_encrypted_event_to_jsonrpc_request(&request_event, &user_keypair)
        else {
            return self.poll_next(cx);
        };

        let (response_sender, response_receiver) = futures::channel::oneshot::channel();

        tokio::spawn(async move {
            response_receiver
                .then(|response| async {
                    if let Ok(response) = response {
                        let response_event = jsonrpc_response_to_nip04_encrypted_event(
                            request_event_kind,
                            &response,
                            request_event_author,
                            &user_keypair,
                        )
                        .unwrap();
                        response_event_sender.send(response_event).unwrap();
                    }
                })
                .await;
        });

        Poll::Ready(Some(((request, user_secret_key), response_sender)))
    }
}

impl AsRef<SingleOrBatch<JsonRpcRequest>> for (SingleOrBatch<JsonRpcRequest>, SecretKey) {
    fn as_ref(&self) -> &SingleOrBatch<JsonRpcRequest> {
        &self.0
    }
}

impl JsonRpcServerTransport<(SingleOrBatch<JsonRpcRequest>, SecretKey)> for Nip55ServerTransport {}
