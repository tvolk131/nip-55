use crate::{
    json_rpc::{
        JsonRpcError, JsonRpcErrorCode, JsonRpcRequest, JsonRpcResponse, JsonRpcResponseData,
        SingleOrBatch,
    },
    stream_helper::map_sender,
    KeyManager, Nip55Client, Nip55ServerStream, UdsClientError,
};
use futures::StreamExt;
use nostr_sdk::{nips::nip46, Keys, PublicKey};
use nostr_sdk::{Kind, SecretKey};
use serde_json::Value;
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// NIP-46 client that can make requests to a NIP-46 server running over NIP-55.
pub struct Nip46OverNip55Client {
    client: Nip55Client,
}

impl Nip46OverNip55Client {
    /// Create a new NIP-46 client that will communicate with a NIP-46 server over NIP-55 on the specified Unix domain socket address.
    pub fn new(uds_address: impl Into<String>) -> Self {
        Self {
            client: Nip55Client::new(uds_address),
        }
    }

    /// Sign an event using the NIP-46 server over NIP-55.
    pub async fn sign_event(
        &self,
        unsigned_event: nostr_sdk::UnsignedEvent,
        user_pubkey: PublicKey,
    ) -> Result<nostr_sdk::Event, Nip46OverNip55ClientError> {
        let request = nip46::Request::SignEvent(unsigned_event);
        let response = self.send_request(&request, user_pubkey).await?;
        match response {
            nip46::ResponseResult::SignEvent(signed_event) => Ok(signed_event),
            _ => Err(Nip46OverNip55ClientError::UdsClientError(
                UdsClientError::MalformedResponse,
            )),
        }
    }

    async fn send_request(
        &self,
        request: &nip46::Request,
        user_pubkey: PublicKey,
    ) -> Result<nip46::ResponseResult, Nip46OverNip55ClientError> {
        let json_rpc_request = request.try_into().map_err(|_| {
            Nip46OverNip55ClientError::UdsClientError(UdsClientError::RequestSerializationError)
        })?;

        let SingleOrBatch::Single(response) = self
            .client
            .send_request(
                Kind::NostrConnect,
                &SingleOrBatch::Single(json_rpc_request),
                user_pubkey,
            )
            .await
            .map_err(Nip46OverNip55ClientError::UdsClientError)?
        else {
            return Err(Nip46OverNip55ClientError::UdsClientError(
                UdsClientError::MalformedResponse,
            ));
        };

        if let JsonRpcResponseData::Error { error } = response.data() {
            return Err(Nip46OverNip55ClientError::JsonRpcError(error.clone()));
        }

        (&response).try_into().map_err(|_| {
            Nip46OverNip55ClientError::UdsClientError(UdsClientError::MalformedResponse)
        })
    }
}

/// Error that can occur when communicating with a NIP-46 server over NIP-55.
#[derive(Debug)]
pub enum Nip46OverNip55ClientError {
    /// A transport-level error occurred.
    UdsClientError(UdsClientError),

    /// The NIP-46 server returned an error response.
    JsonRpcError(JsonRpcError),
}

pub struct Nip46OverNip55ServerStream {
    #[allow(clippy::type_complexity)]
    stream: Pin<
        Box<
            dyn futures::Stream<
                    Item = (
                        (Vec<nip46::Request>, PublicKey),
                        futures::channel::oneshot::Sender<Nip46RequestApproval>,
                    ),
                > + Send,
        >,
    >,
}

impl futures::Stream for Nip46OverNip55ServerStream {
    type Item = (
        Vec<nip46::Request>,
        PublicKey,
        futures::channel::oneshot::Sender<Nip46RequestApproval>,
    );

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.stream.poll_next_unpin(cx).map(|next_item_or| {
            next_item_or.map(|next_item| (next_item.0 .0, next_item.0 .1, next_item.1))
        })
    }
}

impl Nip46OverNip55ServerStream {
    /// Start a new NIP-46 server with NIP-55 as the transport that will listen for incoming connections at the specified Unix domain socket address.
    pub fn start(
        uds_address: impl Into<String>,
        key_manager: Arc<dyn KeyManager>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            stream: Box::pin(
                Nip55ServerStream::start::<(SingleOrBatch<JsonRpcRequest>, SecretKey)>(
                    uds_address,
                    key_manager,
                )?
                .map(|(request, secret_key, response_sender)| {
                    (
                        match request.clone() {
                            SingleOrBatch::Single(request) =>
                            // TODO: Ensure there is only one response and return an error if there is more than one.
                            // Also don't panic if there is no response.
                            {
                                handle_request_array((vec![request], secret_key.clone()))
                            }

                            SingleOrBatch::Batch(requests) =>
                            // TODO: Ensure the order and number of responses matches the order and number of requests.
                            {
                                handle_request_array((requests, secret_key.clone()))
                            }
                        },
                        map_sender(response_sender, move |nip_46_request_approval| {
                            match nip_46_request_approval {
                                Nip46RequestApproval::Approve => request.map(|json_rpc_request| {
                                    let (nip46_request, id): (nip46::Request, String) =
                                        (&json_rpc_request).try_into().unwrap();

                                    let nip46_response = match nip46_request {
                                        nip46::Request::SignEvent(unsigned_event) => {
                                            nip46::ResponseResult::SignEvent(
                                                unsigned_event
                                                    .sign(&Keys::new(secret_key.clone()))
                                                    .unwrap(),
                                            )
                                        }
                                        // TODO: Implement the rest of the NIP-46 methods.
                                        _ => {
                                            return JsonRpcResponseData::Error {
                                                error: JsonRpcError::new(
                                                    JsonRpcErrorCode::MethodNotFound,
                                                    "Method not implemented".to_string(),
                                                    None,
                                                ),
                                            };
                                        }
                                    };

                                    (&nip46_response, &id).try_into().unwrap()
                                }),
                                Nip46RequestApproval::Reject => {
                                    request.map(|_| JsonRpcResponseData::Error {
                                        error: JsonRpcError::new(
                                            JsonRpcErrorCode::InternalError,
                                            "Batch request rejected".to_string(),
                                            None,
                                        ),
                                    })
                                }
                            }
                        }),
                    )
                }),
            ),
        })
    }
}

/// Approval or rejection of a NIP-46 request. Used in the server to determine whether to handle requests or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nip46RequestApproval {
    Approve,
    Reject,
}

fn handle_request_array(
    requests: (Vec<JsonRpcRequest>, SecretKey),
) -> (Vec<nostr_sdk::nips::nip46::Request>, PublicKey) {
    let nip46_requests: Vec<nostr_sdk::nips::nip46::Request> = requests
        .0
        .into_iter()
        .filter_map(|request| (&request).try_into().ok())
        .map(|(nip46_request, _nip46_request_id)| nip46_request)
        .collect();

    let secp = nostr_sdk::secp256k1::Secp256k1::new();

    let public_key: PublicKey = requests.1.x_only_public_key(&secp).0.into();

    (nip46_requests, public_key)
}

impl TryInto<JsonRpcRequest> for &nip46::Request {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<JsonRpcRequest, Self::Error> {
        // TODO: Remove this clone.
        let object_json = match serde_json::json!(nip46::Message::request(self.clone())) {
            Value::Object(mut object) => {
                object.insert("jsonrpc".to_string(), "2.0".into());
                object
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Failed to convert NIP-46 request to JSON-RPC request"
                ));
            }
        };

        Ok(serde_json::from_value(Value::Object(object_json))?)
    }
}

impl TryFrom<&JsonRpcRequest> for (nip46::Request, String) {
    type Error = anyhow::Error;

    fn try_from(value: &JsonRpcRequest) -> Result<Self, Self::Error> {
        let message: nip46::Message = serde_json::from_value(serde_json::json!(value))?;
        let request_id = message.id().to_string();
        Ok((message.to_request()?, request_id))
    }
}

impl TryInto<JsonRpcResponseData> for (&nip46::ResponseResult, &String) {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<JsonRpcResponseData, anyhow::Error> {
        // TODO: Remove this clone.
        Ok(serde_json::from_value(serde_json::json!(
            nip46::Message::response(self.1, Some(self.0.clone()), None)
        ))?)
    }
}

impl TryFrom<&JsonRpcResponse> for nip46::ResponseResult {
    type Error = anyhow::Error;

    fn try_from(value: &JsonRpcResponse) -> Result<Self, anyhow::Error> {
        let message: nip46::Message = serde_json::from_value(serde_json::json!(value))?;
        match message {
            nip46::Message::Response { result, error, .. } => match (result, error) {
                (Some(result), None) => Ok(result),
                (None, Some(error)) => Err(anyhow::anyhow!(error)),
                _ => Err(anyhow::anyhow!("Invalid NIP-46 response")),
            },
            nip46::Message::Request { .. } => Err(anyhow::anyhow!("Invalid NIP-46 response")),
        }
    }
}

// TODO: Currently we're only testing the happy path. Add more tests to cover error/edge cases.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::KeyManager;
    use async_trait::async_trait;
    use nostr_sdk::Keys;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::Mutex;

    struct MockKeyManager {
        keys: Arc<Mutex<HashMap<PublicKey, SecretKey>>>,
    }

    impl Default for MockKeyManager {
        fn default() -> Self {
            Self {
                keys: Arc::new(Mutex::new(HashMap::new())),
            }
        }
    }

    #[async_trait]
    impl KeyManager for MockKeyManager {
        fn get_secret_key(&self, public_key: &PublicKey) -> Option<SecretKey> {
            self.keys.lock().unwrap().get(public_key).cloned()
        }
    }

    impl MockKeyManager {
        fn new() -> Self {
            Self::default()
        }

        fn new_with_single_key(secret_key: SecretKey) -> Self {
            let key_manager = Self::new();
            key_manager.add_key(secret_key);
            key_manager
        }

        fn add_key(&self, secret_key: SecretKey) {
            self.keys.lock().unwrap().insert(
                PublicKey::from(
                    secret_key
                        .x_only_public_key(&nostr_sdk::secp256k1::Secp256k1::new())
                        .0,
                ),
                secret_key,
            );
        }
    }

    #[tokio::test]
    async fn test_nip46_over_nip55() {
        let keypair = Keys::generate();
        let key_manager =
            MockKeyManager::new_with_single_key(keypair.secret_key().unwrap().clone());

        // Since we're starting the server in a separate task, we need to wait for it to start.
        let (server_started_sender, server_started_receiver) = futures::channel::oneshot::channel();

        tokio::task::spawn(async {
            let mut foo = Nip46OverNip55ServerStream::start(
                "/tmp/test.sock".to_string(),
                Arc::new(key_manager),
            )
            .expect("Failed to start NIP-46 over NIP-55 server");

            server_started_sender.send(()).unwrap();

            while let Some((_request_list, _public_key, response_sender)) = foo.next().await {
                response_sender.send(Nip46RequestApproval::Approve).unwrap();
            }
        });

        server_started_receiver.await.unwrap();

        let client = Nip46OverNip55Client::new("/tmp/test.sock".to_string());

        let unsigned_event = nostr_sdk::EventBuilder::new(Kind::TextNote, "example text", None)
            .to_unsigned_event(keypair.public_key());

        let signed_event = client
            .sign_event(unsigned_event, keypair.public_key())
            .await
            .expect("Failed to send NIP-46 request");

        signed_event
            .verify()
            .expect("Failed to verify signed event");
        assert_eq!(signed_event.kind, Kind::TextNote);
        assert_eq!(signed_event.content, "example text");
    }
}
