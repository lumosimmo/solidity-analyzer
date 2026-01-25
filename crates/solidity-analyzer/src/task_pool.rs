use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::task::JoinHandle;

#[derive(Clone, Default)]
pub struct TaskPool;

impl TaskPool {
    pub fn new() -> Self {
        Self
    }

    pub fn spawn<F, T>(&self, f: F) -> Task<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        Task {
            handle: tokio::task::spawn_blocking(f),
        }
    }
}

pub struct Task<T> {
    handle: JoinHandle<T>,
}

impl<T> Future for Task<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.handle).poll(cx)
    }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
