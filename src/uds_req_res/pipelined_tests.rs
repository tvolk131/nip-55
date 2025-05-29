use super::pipelined_client::{PipelinedUnixDomainSocketClientTransport, PipelinedUdsClientError};
use super::pipelined_server::PipelinedUnixDomainSocketServerTransport;
use crate::json_rpc::{JsonRpcId, JsonRpcRequest, JsonRpcResponse, JsonRpcResponseData, SingleOrBatch};
use futures::StreamExt;
use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;

    fn get_random_uds_address() -> String {
        format!("/tmp/test-pipelined-{}.sock", uuid::Uuid::new_v4())
    }

    #[tokio::test]
    async fn test_pipelined_single_request_response() {
        let uds_address = get_random_uds_address();
        let uds_address_clone = uds_address.clone();

        // Start the pipelined server
        let server_transport = PipelinedUnixDomainSocketServerTransport::<
            SingleOrBatch<JsonRpcRequest>,
            SingleOrBatch<JsonRpcResponse>,
        >::connect_and_start(uds_address_clone)
        .expect("Failed to start pipelined server");

        // Spawn a task to handle server requests
        let server_handle = tokio::spawn(async move {
            let mut server_stream = server_transport;
            while let Some((request, response_sender)) = server_stream.next().await {
                // Echo back a success response
                let response = match request {
                    SingleOrBatch::Single(req) => {
                        SingleOrBatch::Single(JsonRpcResponse::new(
                            JsonRpcResponseData::Success {
                                result: serde_json::json!("pong"),
                            },
                            req.id().clone(),
                        ))
                    }
                    SingleOrBatch::Batch(reqs) => {
                        let responses = reqs
                            .iter()
                            .map(|req| {
                                JsonRpcResponse::new(
                                    JsonRpcResponseData::Success {
                                        result: serde_json::json!("pong"),
                                    },
                                    req.id().clone(),
                                )
                            })
                            .collect();
                        SingleOrBatch::Batch(responses)
                    }
                };
                let _ = response_sender.send(response);
            }
        });

        // Give the server a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Create the pipelined client
        let client = PipelinedUnixDomainSocketClientTransport::new(uds_address);

        // Create a test request
        let request = SingleOrBatch::Single(JsonRpcRequest::new(
            "ping".to_string(),
            None,
            JsonRpcId::String("test-1".to_string()),
        ));

        // Send the request
        let response: SingleOrBatch<JsonRpcResponse> = client
            .send_pipelined_request(request)
            .await
            .expect("Failed to send pipelined request");

        // Verify the response
        match response {
            SingleOrBatch::Single(resp) => {
                match resp.data() {
                    JsonRpcResponseData::Success { result } => {
                        assert_eq!(result, &serde_json::json!("pong"));
                    }
                    _ => panic!("Expected success response"),
                }
            }
            _ => panic!("Expected single response"),
        }

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_pipelined_multiple_concurrent_requests() {
        let uds_address = get_random_uds_address();
        let uds_address_clone = uds_address.clone();

        // Start the pipelined server
        let server_transport = PipelinedUnixDomainSocketServerTransport::<
            SingleOrBatch<JsonRpcRequest>,
            SingleOrBatch<JsonRpcResponse>,
        >::connect_and_start(uds_address_clone)
        .expect("Failed to start pipelined server");

        // Spawn a task to handle server requests
        let server_handle = tokio::spawn(async move {
            let mut server_stream = server_transport;
            while let Some((request, response_sender)) = server_stream.next().await {
                // Echo back a success response
                let response = match request {
                    SingleOrBatch::Single(req) => {
                        SingleOrBatch::Single(JsonRpcResponse::new(
                            JsonRpcResponseData::Success {
                                result: serde_json::json!(format!("response-to-{:?}", req.id())),
                            },
                            req.id().clone(),
                        ))
                    }
                    SingleOrBatch::Batch(_) => {
                        panic!("Not expecting batch requests in this test");
                    }
                };
                let _ = response_sender.send(response);
            }
        });

        // Give the server a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Create the pipelined client
        let client = Arc::new(PipelinedUnixDomainSocketClientTransport::new(uds_address));

        // Send multiple concurrent requests
        let mut handles = Vec::new();
        for i in 0..5 {
            let client_clone = client.clone();
            let handle = tokio::spawn(async move {
                let request = SingleOrBatch::Single(JsonRpcRequest::new(
                    "ping".to_string(),
                    None,
                    JsonRpcId::String(format!("test-{}", i)),
                ));

                let response: SingleOrBatch<JsonRpcResponse> = client_clone
                    .send_pipelined_request(request)
                    .await
                    .expect("Failed to send pipelined request");

                (i, response)
            });
            handles.push(handle);
        }

        // Wait for all responses
        let mut results = Vec::new();
        for handle in handles {
            let (i, response) = handle.await.expect("Task failed");
            results.push((i, response));
        }

        // Verify all responses
        assert_eq!(results.len(), 5);
        for (i, response) in results {
            match response {
                SingleOrBatch::Single(resp) => {
                    match resp.data() {
                        JsonRpcResponseData::Success { result } => {
                            let expected = format!("response-to-String(\"test-{}\")", i);
                            assert_eq!(result, &serde_json::json!(expected));
                        }
                        _ => panic!("Expected success response for request {}", i),
                    }
                }
                _ => panic!("Expected single response for request {}", i),
            }
        }

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_pipelined_client_server_not_running() {
        let uds_address = get_random_uds_address();

        // Create the pipelined client without starting the server
        let client = PipelinedUnixDomainSocketClientTransport::new(uds_address);

        // Create a test request
        let request = SingleOrBatch::Single(JsonRpcRequest::new(
            "ping".to_string(),
            None,
            JsonRpcId::String("test-1".to_string()),
        ));

        // Try to send the request - should fail
        let result: Result<SingleOrBatch<JsonRpcResponse>, PipelinedUdsClientError> = client
            .send_pipelined_request(request)
            .await;

        // Verify it fails with ServerNotRunning error
        assert!(matches!(result, Err(PipelinedUdsClientError::ServerNotRunning)));
    }
}