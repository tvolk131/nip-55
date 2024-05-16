use crate::{
    json_rpc::{
        JsonRpcError, JsonRpcErrorCode, JsonRpcRequest, JsonRpcResponse, JsonRpcResponseData,
        JsonRpcServerHandler,
    },
    KeyManager, Nip55Client, Nip55Server, UdsClientError,
};
use async_trait::async_trait;
use nostr_sdk::{nips::nip46, Keys, PublicKey};
use nostr_sdk::{Kind, SecretKey};
use serde_json::Value;

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

        let response = self
            .client
            .send_request(Kind::NostrConnect, &json_rpc_request, user_pubkey)
            .await
            .map_err(Nip46OverNip55ClientError::UdsClientError)?;

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

/// Server that can handle NIP-46 requests over NIP-55.
pub struct Nip46OverNip55Server {
    server: Nip55Server,
}

impl Nip46OverNip55Server {
    /// Start a new NIP-46 server with NIP-55 as the trasnsport that will listen for incoming connections at the specified Unix domain socket address.
    pub fn start(
        uds_address: impl Into<String>,
        key_manager: Box<dyn KeyManager>,
        request_approver: Box<dyn Nip46RequestApprover>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            server: Nip55Server::start(
                uds_address,
                key_manager,
                Nip46OverNip55ServerHandler { request_approver },
            )?,
        })
    }

    /// Stop the NIP-46 server and the underlying Unix domain socket server.
    /// Note: Dropping the server will also stop it.
    pub fn stop(self) {
        self.server.stop();
    }
}

/// Trait to approve or reject NIP-46 requests received by the server.
#[async_trait]
pub trait Nip46RequestApprover: Send + Sync {
    /// Approve or reject a batch of NIP-46 requests received by the server.
    async fn handle_batch_request(
        &self,
        requests: Vec<(nip46::Request, PublicKey)>,
    ) -> Nip46RequestApproval;
}

/// A simple request approver that either always approves or always rejects requests.
pub struct StaticRequestApprover {
    approval: Nip46RequestApproval,
}

impl StaticRequestApprover {
    /// Create a new `StaticRequestApprover` that will always immediately approve requests.
    pub fn always_approve() -> Self {
        Self {
            approval: Nip46RequestApproval::Approve,
        }
    }

    /// Create a new `StaticRequestApprover` that will always immediately reject requests.
    pub fn always_reject() -> Self {
        Self {
            approval: Nip46RequestApproval::Reject,
        }
    }
}

#[async_trait]
impl Nip46RequestApprover for StaticRequestApprover {
    async fn handle_batch_request(
        &self,
        _requests: Vec<(nip46::Request, PublicKey)>,
    ) -> Nip46RequestApproval {
        self.approval
    }
}

/// Approval or rejection of a NIP-46 request. Used in the server to determine whether to handle requests or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nip46RequestApproval {
    Approve,
    Reject,
}

struct Nip46OverNip55ServerHandler {
    request_approver: Box<dyn Nip46RequestApprover>,
}

#[async_trait]
impl JsonRpcServerHandler<(JsonRpcRequest, SecretKey)> for Nip46OverNip55ServerHandler {
    async fn handle_batch_request(
        &self,
        requests: Vec<(JsonRpcRequest, SecretKey)>,
    ) -> Vec<JsonRpcResponseData> {
        let nip46_requests: Vec<Option<(nip46::Request, SecretKey, String)>> = requests
            .into_iter()
            .map(|(request, user_secret_key)| {
                (&request)
                    .try_into()
                    .ok()
                    .map(|(nip46_request, nip46_request_id)| {
                        (nip46_request, user_secret_key, nip46_request_id)
                    })
            })
            .collect();

        let secp = nostr_sdk::secp256k1::Secp256k1::new();

        let approval = self
            .request_approver
            .handle_batch_request(
                nip46_requests
                    .iter()
                    .flatten()
                    .cloned()
                    .map(|(nip46_request, user_secret_key, _nip46_request_id)| {
                        (
                            nip46_request,
                            user_secret_key.x_only_public_key(&secp).0.into(),
                        )
                    })
                    .collect(),
            )
            .await;

        nip46_requests
            .into_iter()
            .map(|request_with_data_or| {
                let (nip46_request, user_secret_key, nip46_request_id) = match request_with_data_or
                {
                    Some(request_with_data) => request_with_data,
                    None => {
                        return JsonRpcResponseData::Error {
                            error: JsonRpcError::new(
                                JsonRpcErrorCode::InvalidRequest,
                                "Request is not a valid NIP-46 request".to_string(),
                                None,
                            ),
                        }
                    }
                };

                if let Nip46RequestApproval::Reject = approval {
                    return JsonRpcResponseData::Error {
                        error: JsonRpcError::new(
                            JsonRpcErrorCode::InternalError,
                            "Batch request rejected".to_string(),
                            None,
                        ),
                    };
                }

                let nip46_response = match nip46_request {
                    nip46::Request::SignEvent(unsigned_event) => nip46::ResponseResult::SignEvent(
                        unsigned_event.sign(&Keys::new(user_secret_key)).unwrap(),
                    ),
                    // TODO: Implement the rest of the NIP-46 methods.
                    _ => {
                        return JsonRpcResponseData::Error {
                            error: JsonRpcError::new(
                                JsonRpcErrorCode::MethodNotFound,
                                "Method not implemented".to_string(),
                                None,
                            ),
                        }
                    }
                };

                match (&nip46_response, &nip46_request_id).try_into() {
                    Ok(response) => response,
                    Err(_) => JsonRpcResponseData::Error {
                        error: JsonRpcError::new(
                            JsonRpcErrorCode::InternalError,
                            "Failed to convert NIP-46 response to JSON-RPC response".to_string(),
                            None,
                        ),
                    },
                }
            })
            .collect()
    }
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
            _ => Err(anyhow::anyhow!("Invalid NIP-46 response")),
        }
    }
}

// TODO: Currently we're only testing the happy path. Add more tests to cover error/edge cases.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::KeyManager;
    use async_trait::async_trait;
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
        let server = Nip46OverNip55Server::start(
            "/tmp/test.sock".to_string(),
            Box::new(key_manager),
            Box::new(StaticRequestApprover::always_approve()),
        )
        .expect("Failed to start NIP-46 over NIP-55 server");

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

        server.stop();
    }
}
