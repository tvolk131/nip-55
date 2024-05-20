use nostr_sdk::Event;
use serde::{de::DeserializeOwned, Serialize};

pub mod single_req_res;

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

/// Error that can occur when communicating with a Unix domain socket server.
#[derive(Clone, Debug, PartialEq)]
pub enum UdsClientError {
    /// A Unix domain socket server is not running on the specified address.
    ServerNotRunning,

    /// An I/O error occurred while writing to or reading from the Unix domain socket.
    UdsSocketError,

    /// An error occurred while serializing the request.
    RequestSerializationError,

    /// Received a response from the server that that cannot be parsed.
    MalformedResponse,
}

impl std::fmt::Display for UdsClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UdsClientError::ServerNotRunning => {
                write!(f, "Unix domain socket server not running.")
            }
            UdsClientError::UdsSocketError => {
                write!(f, "Error writing to or reading from Unix domain socket.")
            }
            UdsClientError::RequestSerializationError => {
                write!(f, "Error serializing the request.")
            }
            UdsClientError::MalformedResponse => {
                write!(
                    f,
                    "Received a response from the server that that cannot be parsed."
                )
            }
        }
    }
}

impl std::error::Error for UdsClientError {}
