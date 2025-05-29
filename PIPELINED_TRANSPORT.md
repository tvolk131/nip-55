# Pipelined Requests and Responses

This crate now supports pipelined JSON-RPC requests over Unix domain sockets, implementing [Option 3](https://www.simple-is-better.org/json-rpc/transport_sockets.html) from the JSON-RPC transport specification.

## Overview

Pipelining allows multiple requests to be sent over the same connection without waiting for individual responses. Responses can be received in any order, making it more efficient for high-throughput scenarios.

## Key Components

### PipelinedUnixDomainSocketClientTransport

The client transport maintains a persistent connection and tracks in-flight requests:

```rust
use nip_55::uds_req_res::pipelined_client::PipelinedUnixDomainSocketClientTransport;
use nip_55::json_rpc::{JsonRpcId, JsonRpcRequest, JsonRpcResponse, SingleOrBatch};

// Create a pipelined client
let client = PipelinedUnixDomainSocketClientTransport::new("/path/to/socket.sock");

// Create a request with a unique ID (required for response correlation)
let request = SingleOrBatch::Single(JsonRpcRequest::new(
    "get_public_key".to_string(),
    None,
    JsonRpcId::String("request-1".to_string()),
));

// Send the request - multiple requests can be sent concurrently
let response: SingleOrBatch<JsonRpcResponse> = client
    .send_pipelined_request(request)
    .await?;
```

### PipelinedUnixDomainSocketServerTransport

The server transport handles multiple concurrent connections with persistent request/response cycles:

```rust
use nip_55::uds_req_res::pipelined_server::PipelinedUnixDomainSocketServerTransport;
use nip_55::json_rpc::{JsonRpcRequest, JsonRpcResponse, SingleOrBatch};
use futures::StreamExt;

// Start the pipelined server
let mut server = PipelinedUnixDomainSocketServerTransport::<
    SingleOrBatch<JsonRpcRequest>,
    SingleOrBatch<JsonRpcResponse>,
>::connect_and_start("/path/to/socket.sock")?;

// Handle requests in a loop
while let Some((request, response_sender)) = server.next().await {
    // Process the request and send back a response
    let response = process_request(request).await;
    let _ = response_sender.send(response);
}
```

## Benefits

1. **Higher Throughput**: Multiple requests can be processed concurrently without connection overhead
2. **Lower Latency**: No need to wait for each response before sending the next request
3. **Efficient Resource Usage**: Single persistent connection instead of multiple short-lived connections
4. **Out-of-Order Responses**: Responses are correlated by request ID, allowing flexible processing

## Requirements

- All requests must have unique IDs for proper response correlation
- Clients must handle response routing based on request IDs
- The server must support persistent connections and continuous request processing

## Backward Compatibility

The existing `UnixDomainSocketClientTransport` and `UnixDomainSocketServerTransport` continue to work unchanged for applications that don't need pipelining.