use tokio::sync::watch;

pub(crate) async fn wait_published<T: Clone>(
    mut receiver: watch::Receiver<Option<T>>,
) -> Result<T, watch::error::RecvError> {
    loop {
        if let Some(value) = receiver.borrow_and_update().clone() {
            return Ok(value);
        }
        receiver.changed().await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        pin::Pin,
        task::{Context, Poll, Waker},
    };

    fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn already_published_value_is_ready_on_first_poll() {
        let (_publisher, receiver) = watch::channel(Some(42));
        let mut waiting = Box::pin(wait_published(receiver));

        assert!(matches!(poll_once(waiting.as_mut()), Poll::Ready(Ok(42))));
    }

    #[test]
    fn pending_wait_observes_later_publication() {
        let (publisher, receiver) = watch::channel(None);
        let mut waiting = Box::pin(wait_published(receiver));

        assert!(matches!(poll_once(waiting.as_mut()), Poll::Pending));
        publisher.send_replace(Some(42));
        assert!(matches!(poll_once(waiting.as_mut()), Poll::Ready(Ok(42))));
    }

    #[test]
    fn closure_before_publication_is_an_error() {
        let (publisher, receiver) = watch::channel(None::<u32>);
        drop(publisher);
        let mut waiting = Box::pin(wait_published(receiver));

        assert!(matches!(poll_once(waiting.as_mut()), Poll::Ready(Err(_))));
    }
}
