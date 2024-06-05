use futures::channel::{mpsc, oneshot};
use futures::SinkExt;
use nostr_relay_pool::{RelayPoolNotification, RelaySendOptions, SubscribeOptions};
use nostr_sdk::{Filter, Keys, Kind, PublicKey, RelayMessage, RelayPool};
use std::pin::Pin;

use crate::json_rpc::{JsonRpcRequest, JsonRpcResponse, JsonRpcServerTransport, SingleOrBatch};
use crate::nip04_jsonrpc::{
    jsonrpc_response_to_nip04_encrypted_event, nip04_encrypted_event_to_jsonrpc_request,
};

impl AsRef<Self> for SingleOrBatch<JsonRpcRequest> {
    fn as_ref(&self) -> &Self {
        self
    }
}

pub struct JsonRpcServerRelayTransport {
    relay_task_handle: tokio::task::JoinHandle<()>,
    rpc_receiver: mpsc::Receiver<(
        SingleOrBatch<JsonRpcRequest>,
        oneshot::Sender<SingleOrBatch<JsonRpcResponse>>,
    )>,
}

impl JsonRpcServerTransport<SingleOrBatch<JsonRpcRequest>> for JsonRpcServerRelayTransport {}

impl std::ops::Drop for JsonRpcServerRelayTransport {
    fn drop(&mut self) {
        // Abort the relay task, since it will loop forever otherwise.
        self.relay_task_handle.abort();
    }
}

impl JsonRpcServerRelayTransport {
    /// Create a new `JsonRpcServerRelayTransport` and start listening for incoming
    /// requests from the specified relays. **MUST** be called from within a tokio runtime.
    pub async fn connect_and_start(
        relay_pool: RelayPool,
        server_keypair: Keys,
        kind: Kind,
    ) -> std::io::Result<Self> {
        // Queue for incoming requests to the server.
        let (rpc_sender, rpc_receiver) = mpsc::channel(1024);

        let transport_subscription_id = relay_pool
            // TODO: Add a `since` and store the last event ID to avoid replaying events.
            .subscribe(
                vec![Filter::new().kind(kind).pubkey(server_keypair.public_key())],
                SubscribeOptions::default(),
            )
            .await;

        let mut notification_receiver = relay_pool.notifications();

        let relay_task_handle = tokio::spawn(async move {
            loop {
                let mut rpc_sender_clone = rpc_sender.clone();

                if let Ok(notification) = notification_receiver.recv().await {
                    let request_event = match notification {
                        RelayPoolNotification::Event {
                            relay_url: _,
                            subscription_id,
                            event,
                        }
                        | RelayPoolNotification::Message {
                            relay_url: _,
                            message:
                                RelayMessage::Event {
                                    subscription_id,
                                    event,
                                },
                        } => {
                            if subscription_id != transport_subscription_id {
                                continue;
                            }
                            *event
                        }
                        _ => continue,
                    };

                    // TODO: Grab the task handle and cancel it when the server is dropped.
                    let server_keypair_clone = server_keypair.clone();
                    let relay_pool_clone = relay_pool.clone();
                    tokio::spawn(async move {
                        let client_pubkey = request_event.pubkey;

                        let Ok(request) = nip04_encrypted_event_to_jsonrpc_request(
                            &request_event,
                            &server_keypair_clone,
                        ) else {
                            panic!()

                            // TODO: Uncomment this.
                            // return Self::send_response_to_relays(
                            //     kind,
                            //     &server_keypair_clone,
                            //     &relay_pool_clone,
                            //     Event::request_parse_error_response(),
                            //     client_pubkey,
                            // )
                            // .await;
                        };

                        let (tx, rx) = oneshot::channel();
                        // TODO: Remove this unwrap. For now it's safe because the receiver will only be dropped when the server is dropped.
                        rpc_sender_clone.send((request, tx)).await.unwrap();
                        if let Ok(response) = rx.await {
                            Self::send_response_to_relays(
                                kind,
                                &server_keypair_clone,
                                &relay_pool_clone,
                                response,
                                client_pubkey,
                            )
                            .await?;
                        }

                        Ok::<(), anyhow::Error>(())
                    });
                }
            }
        });

        Ok(Self {
            relay_task_handle,
            rpc_receiver,
        })
    }

    /// Send a JSON-RPC response to the client that sent the request through the relay pool.
    async fn send_response_to_relays(
        kind: Kind,
        server_keypair: &Keys,
        relay_pool: &RelayPool,
        response: SingleOrBatch<JsonRpcResponse>,
        client_pubkey: PublicKey,
    ) -> Result<(), anyhow::Error> {
        let response_event = jsonrpc_response_to_nip04_encrypted_event(
            kind,
            &response,
            client_pubkey,
            server_keypair,
        )?;

        relay_pool
            .send_event(
                response_event,
                RelaySendOptions::default().skip_send_confirmation(true),
            )
            .await?;

        Ok(())
    }

    fn project(
        self: Pin<&mut Self>,
    ) -> Pin<
        &mut mpsc::Receiver<(
            SingleOrBatch<JsonRpcRequest>,
            oneshot::Sender<SingleOrBatch<JsonRpcResponse>>,
        )>,
    > {
        unsafe { self.map_unchecked_mut(|x| &mut x.rpc_receiver) }
    }
}

impl futures::Stream for JsonRpcServerRelayTransport {
    type Item = (
        SingleOrBatch<JsonRpcRequest>,
        oneshot::Sender<SingleOrBatch<JsonRpcResponse>>,
    );

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.project().poll_next(cx)
    }
}
