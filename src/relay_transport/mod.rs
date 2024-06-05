pub mod client;
pub mod server;

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::StreamExt;
    use nostr_relay_pool::RelayPool;
    use nostr_sdk::{Keys, Kind};
    use serde_json::Value;

    use crate::json_rpc::{
        JsonRpcId, JsonRpcRequest, JsonRpcResponse, JsonRpcResponseData, SingleOrBatch,
    };

    use super::*;

    // Test server and client using relay.damus.io.
    #[tokio::test]
    async fn test_relay_transport() {
        let relay_pool = RelayPool::new(Default::default());
        relay_pool
            .add_relay("wss://relay.damus.io", Default::default())
            .await
            .expect("Failed to add relay");
        relay_pool
            .add_relay("wss://nos.lol", Default::default())
            .await
            .expect("Failed to add relay");
        relay_pool
            .add_relay("wss://nostr.rocks", Default::default())
            .await
            .expect("Failed to add relay");
        relay_pool
            .add_relay("wss://nostr.orangepill.dev", Default::default())
            .await
            .expect("Failed to add relay");
        relay_pool.connect(None).await;

        let server_keypair = Keys::generate();
        let server_pubkey = server_keypair.public_key();
        let client_keypair = Keys::generate();

        let kind = Kind::Custom(123456);

        let mut server = server::JsonRpcServerRelayTransport::connect_and_start(
            relay_pool.clone(),
            server_keypair,
            kind,
        )
        .await
        .expect("Failed to start server");

        let server_task_handle = tokio::spawn(async move {
            while let Some((request, response_sender)) = server.next().await {
                let response = JsonRpcResponse::new(
                    JsonRpcResponseData::Success {
                        result: Value::String(String::from("Success!")),
                    },
                    JsonRpcId::Null,
                );
                response_sender
                    .send(SingleOrBatch::Single(response))
                    .expect("Failed to send response");
            }
        });

        let client = client::JsonRpcClientRelayTransport::new(
            relay_pool,
            kind,
            client_keypair,
            Duration::from_secs(10),
        );

        let request = JsonRpcRequest::new("test_method".into(), None, JsonRpcId::Null);

        let response = client
            .send_request(SingleOrBatch::Single(request), server_pubkey)
            .await
            .expect("Failed to send request");

        if let SingleOrBatch::Single(response) = response {
            assert_eq!(
                response.data(),
                &JsonRpcResponseData::Success {
                    result: Value::String(String::from("Success!")),
                }
            )
        } else {
            panic!("Expected single response");
        }

        server_task_handle.abort();
    }
}
