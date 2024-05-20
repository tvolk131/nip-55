use super::{
    nip04_jsonrpc::{
        jsonrpc_request_to_nip04_encrypted_event, nip04_encrypted_event_to_jsonrpc_response,
    },
    uds::single_req_res::client::{UdsClientError, UnixDomainSocketClientTransport},
};
use crate::json_rpc::{JsonRpcRequest, JsonRpcResponse, SingleOrBatch};
use nostr_sdk::{Event, Keys, Kind, PublicKey};

/// NIP-55 client that can make requests to a NIP-55 server.
#[derive(Clone)]
pub struct Nip55Client {
    uds_client_transport: UnixDomainSocketClientTransport,
}

// TODO: Support batch requests.
impl Nip55Client {
    pub fn new(uds_address: impl Into<String>) -> Self {
        Self {
            uds_client_transport: UnixDomainSocketClientTransport::new(uds_address),
        }
    }

    pub async fn send_request(
        &self,
        kind: Kind,
        request: &SingleOrBatch<JsonRpcRequest>,
        server_pubkey: PublicKey,
    ) -> Result<SingleOrBatch<JsonRpcResponse>, UdsClientError> {
        let temp_client_keypair = Keys::generate();

        let request_event: Event = jsonrpc_request_to_nip04_encrypted_event(
            kind,
            request,
            &temp_client_keypair,
            server_pubkey,
        )
        .map_err(|_| UdsClientError::RequestSerializationError)?;

        let response_event = self
            .uds_client_transport
            .send_request(request_event)
            .await?;

        nip04_encrypted_event_to_jsonrpc_response(&response_event, &temp_client_keypair)
            .map_err(|_| UdsClientError::MalformedResponse)
    }
}
