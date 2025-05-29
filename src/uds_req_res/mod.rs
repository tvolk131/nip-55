use nostr_sdk::Event;
use serde::{de::DeserializeOwned, Serialize};

use crate::json_rpc::{JsonRpcRequest, JsonRpcResponse, SingleOrBatch};

pub mod client;
pub mod pipelined_client;
pub mod pipelined_server;
#[cfg(test)]
mod pipelined_tests;
pub mod server;

pub trait UdsRequest: Serialize + DeserializeOwned + Send + 'static {}

pub trait UdsResponse: Serialize + DeserializeOwned + Send + 'static {
    /// Create a response representing that the request could not be parsed.
    fn request_parse_error_response() -> Self;
}

impl UdsRequest for Event {}
impl UdsRequest for SingleOrBatch<JsonRpcRequest> {}

impl UdsResponse for Event {
    fn request_parse_error_response() -> Self {
        // TODO: Implement this.
        panic!()
    }
}

impl UdsResponse for SingleOrBatch<JsonRpcResponse> {
    fn request_parse_error_response() -> Self {
        // TODO: Implement this properly with a real parse error response.
        panic!("Parse error response not implemented for SingleOrBatch<JsonRpcResponse>")
    }
}

// TODO: Test that the client and server can communicate with each other.
