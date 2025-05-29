use std::pin::Pin;
use futures::SinkExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

use crate::json_rpc::{JsonRpcRequest, JsonRpcResponse, JsonRpcServerTransport, SingleOrBatch};
use super::{UdsRequest, UdsResponse};

/// A pipelined Unix domain socket server transport that handles multiple
/// requests over persistent connections using StreamDeserializer.
pub struct PipelinedUnixDomainSocketServerTransport<Request: UdsRequest, Response: UdsResponse> {
    uds_task_handle: tokio::task::JoinHandle<()>,
    rpc_receiver:
        futures::channel::mpsc::Receiver<(Request, futures::channel::oneshot::Sender<Response>)>,
    uds_address: String,
}

impl<SingleOrBatchRequest: AsRef<SingleOrBatch<JsonRpcRequest>> + UdsRequest>
    JsonRpcServerTransport<SingleOrBatchRequest>
    for PipelinedUnixDomainSocketServerTransport<SingleOrBatchRequest, SingleOrBatch<JsonRpcResponse>>
{
}

impl<Request: UdsRequest, Response: UdsResponse> std::ops::Drop
    for PipelinedUnixDomainSocketServerTransport<Request, Response>
{
    fn drop(&mut self) {
        // Abort the UDS task, since it will loop forever otherwise.
        self.uds_task_handle.abort();

        // Try to remove the UDS file. If it fails, it's not a big deal.
        let _ = std::fs::remove_file(&self.uds_address);
    }
}

impl<Request: UdsRequest, Response: UdsResponse>
    PipelinedUnixDomainSocketServerTransport<Request, Response>
{
    /// Create a new `PipelinedUnixDomainSocketServerTransport` and start listening for incoming
    /// connections. **MUST** be called from within a tokio runtime.
    pub fn connect_and_start(uds_address: impl Into<String>) -> std::io::Result<Self> {
        let uds_address = uds_address.into();

        if std::path::Path::new(&uds_address).exists() {
            std::fs::remove_file(&uds_address)?;
        }

        // Queue for incoming requests to the server.
        let (rpc_sender, rpc_receiver) = futures::channel::mpsc::channel(1024);

        let listener = UnixListener::bind(&uds_address)?;

        let uds_task_handle = tokio::spawn(async move {            
            loop {
                let mut rpc_sender_clone = rpc_sender.clone();

                if let Ok((socket, _)) = listener.accept().await {
                    // Spawn a task to handle this connection
                    tokio::spawn(async move {
                        let (read_half, mut write_half) = socket.into_split();
                        let mut buf_reader = BufReader::new(read_half);
                        let mut line = String::new();
                        
                        loop {
                            line.clear();
                            match buf_reader.read_line(&mut line).await {
                                Ok(0) => break, // EOF
                                Ok(_) => {
                                    if let Ok(request) = serde_json::from_str::<Request>(&line) {
                                        let (tx, rx) = futures::channel::oneshot::channel();
                                        
                                        // Send the request to be processed
                                        if rpc_sender_clone.send((request, tx)).await.is_err() {
                                            break; // Server shutting down
                                        }
                                        
                                        // Wait for response and send it back over the connection
                                        if let Ok(response) = rx.await {
                                            if let Ok(serialized_response) = serde_json::to_vec(&response) {
                                                let _ = write_half.write_all(&serialized_response).await;
                                                let _ = write_half.write_all(b"\n").await;
                                                let _ = write_half.flush().await;
                                            }
                                        }
                                    } else {
                                        // Invalid request, send error response
                                        let error_response = Response::request_parse_error_response();
                                        if let Ok(serialized_response) = serde_json::to_vec(&error_response) {
                                            let _ = write_half.write_all(&serialized_response).await;
                                            let _ = write_half.write_all(b"\n").await;
                                            let _ = write_half.flush().await;
                                        }
                                    }
                                }
                                Err(_) => break, // Connection error
                            }
                        }
                    });
                }
            }
        });

        Ok(Self {
            uds_task_handle,
            rpc_receiver,
            uds_address,
        })
    }

    fn project(
        self: Pin<&mut Self>,
    ) -> Pin<
        &mut futures::channel::mpsc::Receiver<(
            Request,
            futures::channel::oneshot::Sender<Response>,
        )>,
    > {
        unsafe { self.map_unchecked_mut(|x| &mut x.rpc_receiver) }
    }
}

impl<Request: UdsRequest, Response: UdsResponse> futures::Stream
    for PipelinedUnixDomainSocketServerTransport<Request, Response>
{
    type Item = (Request, futures::channel::oneshot::Sender<Response>);

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.project().poll_next(cx)
    }
}