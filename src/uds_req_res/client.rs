use super::{UdsRequest, UdsResponse};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

#[derive(Clone)]
pub struct UnixDomainSocketClientTransport {
    uds_address: String,
}

impl UnixDomainSocketClientTransport {
    pub fn new(uds_address: impl Into<String>) -> Self {
        Self {
            uds_address: uds_address.into(),
        }
    }

    pub async fn send_request<Request: UdsRequest, Response: UdsResponse>(
        &self,
        request: Request,
    ) -> Result<Response, UdsClientError> {
        let serialized_request =
            serde_json::to_vec(&request).map_err(|_| UdsClientError::RequestSerializationError)?;
        let serialize_response = self.send_and_receive_bytes(serialized_request).await?;
        serde_json::from_slice::<Response>(&serialize_response)
            .map_err(|e| UdsClientError::MalformedResponse(e.into()))
    }

    async fn send_and_receive_bytes(
        &self,
        serialized_request: Vec<u8>,
    ) -> Result<Vec<u8>, UdsClientError> {
        // Open up a UDS connection to the server.
        let mut socket = UnixStream::connect(&self.uds_address)
            .await
            .map_err(|_| UdsClientError::ServerNotRunning)?;

        socket
            .write_all(&serialized_request)
            .await
            .map_err(|_| UdsClientError::UdsSocketError)?;
        socket
            .shutdown()
            .await
            .map_err(|_| UdsClientError::UdsSocketError)?;

        // Read the response from the server.
        // TODO: Add a timeout to this read operation.
        let mut buf = Vec::new();
        socket
            .read_to_end(&mut buf)
            .await
            .map_err(|_| UdsClientError::UdsSocketError)?;
        Ok(buf)
    }
}

/// Error that can occur when communicating with a Unix domain socket server.
#[derive(Debug)]
pub enum UdsClientError {
    /// A Unix domain socket server is not running on the specified address.
    ServerNotRunning,

    /// An I/O error occurred while writing to or reading from the Unix domain socket.
    UdsSocketError,

    /// An error occurred while serializing the request.
    RequestSerializationError,

    /// Received a response from the server that that cannot be parsed.
    MalformedResponse(anyhow::Error),
}

impl std::fmt::Display for UdsClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ServerNotRunning => {
                write!(f, "Unix domain socket server not running.")
            }
            Self::UdsSocketError => {
                write!(f, "Error writing to or reading from Unix domain socket.")
            }
            Self::RequestSerializationError => {
                write!(f, "Error serializing the request.")
            }
            Self::MalformedResponse(malformed_response_error) => {
                write!(
                    f,
                    "Received a response from the server that that cannot be parsed ({malformed_response_error})."
                )
            }
        }
    }
}

impl std::error::Error for UdsClientError {}
