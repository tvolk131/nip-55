use std::time::Duration;

use crate::{
    json_rpc::{JsonRpcRequest, JsonRpcResponse, SingleOrBatch},
    nip04_jsonrpc::{
        jsonrpc_request_to_nip04_encrypted_event, nip04_encrypted_event_to_jsonrpc_response,
    },
};
use nostr_relay_pool::{FilterOptions, RelayPool, RelaySendOptions};
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

        let relay_pool_clone = self.relay_pool.clone();
        let send_event_handle = tokio::spawn(async move {
            relay_pool_clone
                .send_event(
                    request_event.clone(),
                    RelaySendOptions::default().skip_send_confirmation(true),
                )
                .await
        });

        let wait_start = std::time::Instant::now();
        let mut get_response_handles: Vec<
            tokio::task::JoinHandle<Result<SingleOrBatch<JsonRpcResponse>, anyhow::Error>>,
        > = Vec::new();
        while wait_start.elapsed() < self.request_timeout {
            let mut temp_get_response_handles = Vec::new();
            temp_get_response_handles.append(&mut get_response_handles);
            for get_response_handle in temp_get_response_handles {
                if get_response_handle.is_finished() {
                    if let Ok(response) = get_response_handle.await? {
                        // No need to keep sending the request to more relays if we got a response.
                        send_event_handle.abort();

                        return Ok(response);
                    }
                } else {
                    get_response_handles.push(get_response_handle);
                }
            }

            let kind = self.kind;
            let client_keypair = self.client_keypair.clone();
            let relay_pool_clone = self.relay_pool.clone();

            let get_response_handle = tokio::spawn(async move {
                if let Ok(maybe_response_events) = relay_pool_clone
                    .get_events_of(
                        vec![
                            Filter::new()
                                .kind(kind)
                                .author(server_pubkey)
                                .pubkey(client_keypair.public_key()), // TODO: add `.since(request_event.created_at())`.
                        ],
                        Duration::from_secs(2),
                        FilterOptions::default(),
                    )
                    .await
                {
                    if let Some(response) = maybe_response_events.into_iter().find_map(|event| {
                        // TODO: Somehow check the response id and match it with the request id. Non-trivial for batch requests.
                        nip04_encrypted_event_to_jsonrpc_response(&event, &client_keypair).ok()
                    }) {
                        return Ok(response);
                    }
                }

                Err(anyhow::anyhow!("Failed to get response events."))
            });
            get_response_handles.push(get_response_handle);

            tokio::task::yield_now().await;
        }

        // No need to keep sending the request to more relays if we gave up waiting.
        send_event_handle.abort();

        Err(anyhow::anyhow!("Request timed out."))
    }
}
