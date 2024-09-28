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

#[cfg(test)]
mod tests {
    use crate::UdsClientError;

    use super::*;

    use client::UnixDomainSocketClientTransport;
    use futures::StreamExt;
    use server::UnixDomainSocketServerTransport;

    #[tokio::test]
    async fn test_server_drops_response_sender() {
        impl UdsRequest for () {}
        impl UdsResponse for () {
            fn request_parse_error_response() -> Self {
                ()
            }
        }

        // Since we're starting the server in a separate task, we need to wait for it to start.
        let (server_started_sender, server_started_receiver) = futures::channel::oneshot::channel();

        let server_handle = tokio::task::spawn(async {
            let mut foo: UnixDomainSocketServerTransport<(), ()> =
                UnixDomainSocketServerTransport::connect_and_start(
                    "/tmp/test12333.sock".to_string(),
                )
                .expect("Failed to start UDS server");

            server_started_sender.send(()).unwrap();

            while let Some((_request, response_sender)) = foo.next().await {
                drop(response_sender);
            }
        });

        server_started_receiver.await.unwrap();

        let client = UnixDomainSocketClientTransport::new("/tmp/test12333.sock".to_string());

        for _ in 0..10 {
            assert!(matches!(
                client.send_request::<(), ()>(()).await,
                Err(UdsClientError::MalformedResponse(_))
            ));
        }

        server_handle.abort();
    }
}
