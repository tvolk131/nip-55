use super::super::{UdsClientError, UdsRequest, UdsResponse};
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
            .map_err(|_| UdsClientError::MalformedResponse)
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
