use super::nip04_jsonrpc::{
    jsonrpc_response_to_nip04_encrypted_event, nip04_encrypted_event_to_jsonrpc_request,
};
use crate::json_rpc::{JsonRpcServer, JsonRpcServerHandler};
use crate::uds_req_res::UdsResponse;
use crate::{
    json_rpc::{JsonRpcRequest, JsonRpcResponse, JsonRpcServerTransport},
    uds_req_res::server::UnixDomainSocketServerTransport,
};
use futures::{FutureExt, StreamExt};
use nostr_sdk::{Event, Keys, PublicKey, SecretKey};
use std::task::Poll;

pub trait KeyManager: Send + Sync {
    fn get_secret_key(&self, public_key: &PublicKey) -> Option<SecretKey>;
}

/// NIP-55 server that can receive requests from a NIP-55 client.
pub struct Nip55Server {
    server: JsonRpcServer,
}

impl Nip55Server {
    pub fn start(
        uds_address: String,
        key_manager: Box<dyn KeyManager>,
        handler: Box<dyn JsonRpcServerHandler<(JsonRpcRequest, SecretKey)>>,
    ) -> std::io::Result<Self> {
        let transport = Nip55ServerTransport::connect_and_start(uds_address, key_manager)?;
        let server = JsonRpcServer::start(Box::from(transport), handler);
        Ok(Self { server })
    }

    pub fn stop(self) {
        self.server.stop();
    }
}

struct Nip55ServerTransport {
    transport_server: UnixDomainSocketServerTransport<Event, Event>,
    key_manager: Box<dyn KeyManager>,
}

impl Nip55ServerTransport {
    /// Create a new `Nip55ServerTransport` and start listening for incoming
    /// connections. **MUST** be called from within a tokio runtime.
    fn connect_and_start(
        uds_address: String,
        key_manager: Box<dyn KeyManager>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            transport_server: UnixDomainSocketServerTransport::connect_and_start(uds_address)?,
            key_manager,
        })
    }
}

impl futures::Stream for Nip55ServerTransport {
    type Item = (
        (JsonRpcRequest, SecretKey),
        futures::channel::oneshot::Sender<JsonRpcResponse>,
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
            Poll::Ready(None) => return Poll::Pending,
            Poll::Pending => return Poll::Pending,
        };

        let request_event_kind = request_event.kind();
        let request_event_author = request_event.author();

        // TODO: Should we attempt to NIP-04 decrypt the request for all public keys rather than just the first one?
        let user_public_key = match request_event.public_keys().next() {
            Some(user_public_key) => user_public_key,
            None => {
                // TODO: Should we send a response to `response_event_sender`? What secret key should we use to sign it?
                return Poll::Pending;
            }
        };

        let user_secret_key = match self.key_manager.get_secret_key(user_public_key) {
            Some(user_secret_key) => user_secret_key,
            None => {
                // TODO: Should we send a response to `response_event_sender`? What secret key should we use to sign it?
                return Poll::Pending;
            }
        };

        let user_keypair = Keys::new(user_secret_key.clone());

        let request = match nip04_encrypted_event_to_jsonrpc_request(&request_event, &user_keypair)
        {
            Ok(request) => request,
            Err(_) => return Poll::Pending,
        };

        let (response_sender, response_receiver) = futures::channel::oneshot::channel();

        tokio::spawn(async move {
            response_receiver
                .then(|response| async {
                    match response {
                        Ok(response) => {
                            let response_event = jsonrpc_response_to_nip04_encrypted_event(
                                request_event_kind,
                                &response,
                                request_event_author,
                                &user_keypair,
                            )
                            .unwrap();
                            response_event_sender.send(response_event).unwrap();
                        }
                        Err(_) => {
                            let response_event = jsonrpc_response_to_nip04_encrypted_event(
                                request_event_kind,
                                &JsonRpcResponse::internal_error_response(
                                    "Internal error.".to_string(),
                                ),
                                request_event_author,
                                &user_keypair,
                            )
                            .unwrap();
                            response_event_sender.send(response_event).unwrap();
                        }
                    }
                })
                .await;
        });

        Poll::Ready(Some(((request, user_secret_key), response_sender)))
    }
}

impl AsRef<JsonRpcRequest> for (JsonRpcRequest, SecretKey) {
    fn as_ref(&self) -> &JsonRpcRequest {
        &self.0
    }
}

impl JsonRpcServerTransport<(JsonRpcRequest, SecretKey)> for Nip55ServerTransport {}
