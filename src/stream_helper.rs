use std::fmt::Debug;

use futures::channel::oneshot;

pub fn map_sender<TOld: Send + Sync + Debug + 'static, TNew: Send + Sync + 'static>(
    sender: oneshot::Sender<TOld>,
    map: impl FnOnce(TNew) -> TOld + Send + Sync + 'static,
) -> oneshot::Sender<TNew> {
    let (new_sender, new_receiver) = oneshot::channel::<TNew>();

    tokio::spawn(async move {
        if let Ok(value) = new_receiver.await {
            sender.send(map(value)).unwrap();
        }
    });

    new_sender
}
