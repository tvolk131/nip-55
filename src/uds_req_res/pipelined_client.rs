use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncWriteExt, AsyncBufReadExt, BufReader};
use tokio::net::{UnixStream, unix::OwnedWriteHalf};
use tokio::sync::{Mutex, oneshot};

use super::{UdsRequest, UdsResponse};

/// A pipelined Unix domain socket client transport that supports sending multiple
/// requests over the same connection without waiting for individual responses.
#[derive(Clone)]
pub struct PipelinedUnixDomainSocketClientTransport {
    uds_address: String,
    connection: Arc<Mutex<Option<PipelinedConnection>>>,
}

struct PipelinedConnection {
    request_sender: Arc<Mutex<OwnedWriteHalf>>,
    pending_requests: Arc<Mutex<HashMap<String, oneshot::Sender<serde_json::Value>>>>,
    _response_task: tokio::task::JoinHandle<()>,
}

impl PipelinedUnixDomainSocketClientTransport {
    pub fn new(uds_address: impl Into<String>) -> Self {
        Self {
            uds_address: uds_address.into(),
            connection: Arc::new(Mutex::new(None)),
        }
    }

    /// Send a pipelined request. The request must have a unique ID for response correlation.
    pub async fn send_pipelined_request<Request: UdsRequest + serde::Serialize, Response: UdsResponse>(
        &self,
        request: Request,
    ) -> Result<Response, PipelinedUdsClientError> {
        // Serialize request to extract ID for tracking
        let serialized_request = serde_json::to_vec(&request)
            .map_err(|_| PipelinedUdsClientError::RequestSerializationError)?;
        
        let request_value: serde_json::Value = serde_json::from_slice(&serialized_request)
            .map_err(|_| PipelinedUdsClientError::RequestSerializationError)?;
        
        // Extract request ID for tracking response
        let request_id = match request_value.get("id") {
            Some(id) => id.to_string(),
            None => return Err(PipelinedUdsClientError::MissingRequestId),
        };

        // Ensure connection is established
        self.ensure_connection().await?;
        
        let (response_sender, response_receiver) = oneshot::channel();
        
        // Get connection and add pending request
        {
            let connection_guard = self.connection.lock().await;
            let connection = connection_guard.as_ref()
                .ok_or(PipelinedUdsClientError::ConnectionError)?;
            
            // Register pending request
            {
                let mut pending = connection.pending_requests.lock().await;
                pending.insert(request_id, response_sender);
            }
            
            // Send request
            {
                let mut sender = connection.request_sender.lock().await;
                sender.write_all(&serialized_request).await
                    .map_err(|_| PipelinedUdsClientError::UdsSocketError)?;
                sender.write_all(b"\n").await
                    .map_err(|_| PipelinedUdsClientError::UdsSocketError)?;
                sender.flush().await
                    .map_err(|_| PipelinedUdsClientError::UdsSocketError)?;
            }
        }
        
        // Wait for response
        let response_value = response_receiver.await
            .map_err(|_| PipelinedUdsClientError::ResponseChannelClosed)?;
        
        // Deserialize response
        serde_json::from_value::<Response>(response_value)
            .map_err(|e| PipelinedUdsClientError::MalformedResponse(e.into()))
    }

    async fn ensure_connection(&self) -> Result<(), PipelinedUdsClientError> {
        let mut connection_guard = self.connection.lock().await;
        
        if connection_guard.is_none() {
            // Create new connection
            let socket = UnixStream::connect(&self.uds_address).await
                .map_err(|_| PipelinedUdsClientError::ServerNotRunning)?;
            
            let (read_half, write_half) = socket.into_split();
            
            let pending_requests: Arc<Mutex<HashMap<String, oneshot::Sender<serde_json::Value>>>> = Arc::new(Mutex::new(HashMap::new()));
            let pending_requests_clone = pending_requests.clone();
            
            // Start response reading task
            let response_task = tokio::spawn(async move {
                let mut buf_reader = BufReader::new(read_half);
                let mut line = String::new();
                
                loop {
                    line.clear();
                    match buf_reader.read_line(&mut line).await {
                        Ok(0) => break, // EOF
                        Ok(_) => {
                            if let Ok(response_value) = serde_json::from_str::<serde_json::Value>(&line) {
                                // Extract response ID to match with pending request
                                if let Some(id_value) = response_value.get("id") {
                                    let response_id = id_value.to_string();
                                    let mut pending = pending_requests_clone.lock().await;
                                    if let Some(sender) = pending.remove(&response_id) {
                                        let _ = sender.send(response_value);
                                    }
                                }
                            }
                        }
                        Err(_) => break, // Connection error
                    }
                }
            });
            
            *connection_guard = Some(PipelinedConnection {
                request_sender: Arc::new(Mutex::new(write_half)),
                pending_requests,
                _response_task: response_task,
            });
        }
        
        Ok(())
    }
}

/// Error that can occur when communicating with a pipelined Unix domain socket server.
#[derive(Debug)]
pub enum PipelinedUdsClientError {
    /// A Unix domain socket server is not running on the specified address.
    ServerNotRunning,
    
    /// An I/O error occurred while writing to or reading from the Unix domain socket.
    UdsSocketError,
    
    /// An error occurred while serializing the request.
    RequestSerializationError,
    
    /// The request is missing an ID field required for pipelining.
    MissingRequestId,
    
    /// Connection error.
    ConnectionError,
    
    /// Response channel was closed unexpectedly.
    ResponseChannelClosed,
    
    /// Received a response from the server that cannot be parsed.
    MalformedResponse(anyhow::Error),
}

impl std::fmt::Display for PipelinedUdsClientError {
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
            Self::MissingRequestId => {
                write!(f, "Request missing ID field required for pipelining.")
            }
            Self::ConnectionError => {
                write!(f, "Connection error.")
            }
            Self::ResponseChannelClosed => {
                write!(f, "Response channel was closed unexpectedly.")
            }
            Self::MalformedResponse(malformed_response_error) => {
                write!(
                    f,
                    "Received a response from the server that cannot be parsed ({malformed_response_error})."
                )
            }
        }
    }
}

impl std::error::Error for PipelinedUdsClientError {}