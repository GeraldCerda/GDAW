use concurrent_queue::ConcurrentQueue;
use std::sync::Arc;
use std::sync::mpsc::{SendError, TryRecvError};
#[cfg(test)]
use std::sync::mpsc::TrySendError;

pub struct Sender<T>(Arc<ConcurrentQueue<T>>);
pub struct Receiver<T>(Arc<ConcurrentQueue<T>>);
pub type SyncSender<T> = Sender<T>;

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self { Self(Arc::clone(&self.0)) }
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) { sync_channel(256) }

pub fn sync_channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let queue = Arc::new(ConcurrentQueue::bounded(capacity.max(1)));
    (Sender(Arc::clone(&queue)), Receiver(queue))
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        self.0.push(value).map_err(|error| SendError(error.into_inner()))
    }

    #[cfg(test)]
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        self.0.push(value).map_err(|error| TrySendError::Full(error.into_inner()))
    }
}

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Result<T, TryRecvError> { self.0.pop().map_err(|_| TryRecvError::Empty) }
    pub fn try_iter(&self) -> impl Iterator<Item = T> + '_ { std::iter::from_fn(|| self.try_recv().ok()) }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_full_mailbox_returns_ownership_to_the_sender() {
        let (sender, receiver) = sync_channel(1);
        sender.send(Box::new(1)).unwrap();
        let rejected = sender.send(Box::new(2)).unwrap_err().0;
        assert_eq!(*rejected, 2);
        assert_eq!(*receiver.try_recv().unwrap(), 1);
        sender.send(rejected).unwrap();
        assert_eq!(*receiver.try_recv().unwrap(), 2);
    }
}
