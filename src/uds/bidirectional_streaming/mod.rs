pub mod client;
pub mod server;

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use super::*;
    use crate::uds::UdsMessage;
    use futures::StreamExt;

    impl UdsMessage for i32 {}

    #[tokio::test]
    async fn test() {
        let mut server: server::UnixDomainSocketServerTransport<i32, i32> =
            server::UnixDomainSocketServerTransport::connect_and_start("/tmp/bahaha").unwrap();

        let server_received_values: Arc<Mutex<Vec<i32>>> = Arc::new(Mutex::new(Vec::new()));

        let server_received_values_clone = server_received_values.clone();
        let server_handle = tokio::spawn(async move {
            while let Some((mut connection_receiver, mut connection_sender)) = server.next().await {
                let server_received_values_clone = server_received_values_clone.clone();
                tokio::spawn(async move {
                    loop {
                        if let Some(item) = connection_receiver.next().await {
                            server_received_values_clone.lock().await.push(item);
                        } else {
                            // break;
                        }
                        tokio::task::yield_now().await;
                    }
                });
            }
        });

        let mut client: client::UnixDomainSocketClientTransport<i32, i32> =
            client::UnixDomainSocketClientTransport::new("/tmp/bahaha")
                .await
                .unwrap();

        for i in 0..4 {
            client.send_outgoing_message(i).await.unwrap();
        }

        // Sleep for a bit to allow the server to catch up.
        // tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

        assert_eq!(
            server_received_values.lock().await.clone(),
            vec![1, 2, 3, 4]
        );

        // Stop the server.
        server_handle.abort();
    }
}
