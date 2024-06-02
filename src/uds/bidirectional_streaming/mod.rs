pub mod client;
mod reader;
pub mod server;

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use super::*;
    use crate::uds::UdsMessage;
    use futures::StreamExt;

    impl UdsMessage for Value {}

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    // #[tokio::test]
    async fn test() {
        let server_received_values: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

        let server_received_values_clone = server_received_values.clone();
        let server: server::UnixDomainSocketServerTransport<Value, Value> =
            server::UnixDomainSocketServerTransport::connect_and_start(
                "/tmp/bahaha",
                move |mut receiver, sender| {
                    let server_received_values_clone = server_received_values_clone.clone();
                    tokio::spawn(async move {
                        while let Some(item) = receiver.next().await {
                            panic!("INCOMING CONNECTION");
                            server_received_values_clone.lock().await.push(item);
                        }
                    });
                },
            )
            .unwrap();

        let mut client: client::UnixDomainSocketClientTransport<Value, Value> =
            client::UnixDomainSocketClientTransport::new("/tmp/bahaha")
                .await
                .unwrap();

        for i in 0..4 {
            client
                .send_outgoing_message(Value::Array(Vec::new()))
                .await
                .unwrap();
        }

        // drop(client);

        // Sleep for a bit to allow the server to catch up.
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

        assert_eq!(
            server_received_values.lock().await.clone(),
            vec![1, 2, 3, 4]
        );
    }
}
