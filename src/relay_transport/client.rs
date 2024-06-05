use std::time::Duration;

use crate::{
    json_rpc::{JsonRpcRequest, JsonRpcResponse, SingleOrBatch},
    nip04_jsonrpc::{
        jsonrpc_request_to_nip04_encrypted_event, nip04_encrypted_event_to_jsonrpc_response,
    },
};
use nostr_relay_pool::{RelayPool, RelaySendOptions};
use nostr_sdk::{Filter, Keys, Kind, PublicKey};

#[derive(Clone)]
pub struct JsonRpcClientRelayTransport {
    relay_pool: RelayPool,
    kind: Kind,
    client_keypair: Keys,
    request_timeout: Duration,
}

impl JsonRpcClientRelayTransport {
    pub fn new(
        relay_pool: RelayPool,
        kind: Kind,
        client_keypair: Keys,
        request_timeout: Duration,
    ) -> Self {
        Self {
            relay_pool,
            kind,
            client_keypair,
            request_timeout,
        }
    }

    pub async fn send_request(
        &self,
        request: SingleOrBatch<JsonRpcRequest>,
        server_pubkey: PublicKey,
    ) -> Result<SingleOrBatch<JsonRpcResponse>, anyhow::Error> {
        let request_event = jsonrpc_request_to_nip04_encrypted_event(
            self.kind,
            &request,
            &self.client_keypair,
            server_pubkey,
        )?;

        self.relay_pool
            .send_event(
                request_event.clone(),
                RelaySendOptions::default().skip_send_confirmation(true),
            )
            .await?;

        let wait_start = std::time::Instant::now();
        while std::time::Instant::now() - wait_start < self.request_timeout {
            if let Ok(maybe_response_events) = self
                .relay_pool
                .get_events_of(
                    vec![
                        Filter::new()
                            .kind(self.kind)
                            .author(server_pubkey)
                            .pubkey(self.client_keypair.public_key()), // TODO: add ``.since(request_event.created_at())``
                    ],
                    Duration::from_secs(2),
                    Default::default(),
                )
                .await
            {
                if let Some(response) = maybe_response_events.into_iter().find_map(|event| {
                    // TODO: Somehow check the response id and match it with the request id. Non-trivial for batch requests.
                    nip04_encrypted_event_to_jsonrpc_response(&event, &self.client_keypair).ok()
                }) {
                    return Ok(response);
                }
            }
        }

        Err(anyhow::anyhow!("Request timed out."))
    }
}
