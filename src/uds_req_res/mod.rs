use nostr_sdk::Event;
use serde::{de::DeserializeOwned, Serialize};

pub mod client;
pub mod server;

pub trait UdsRequest: Serialize + DeserializeOwned + Send + 'static {}

pub trait UdsResponse: Serialize + DeserializeOwned + Send + 'static {
    /// Create a response representing that the request could not be parsed.
    fn request_parse_error_response() -> Self;
}

impl UdsRequest for Event {}

impl UdsResponse for Event {
    fn request_parse_error_response() -> Self {
        // TODO: Implement this.
        panic!()
    }
}

// TODO: Test that the client and server can communicate with each other.
