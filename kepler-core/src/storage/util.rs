use crate::hash::{Hash, Hasher};
use core::pin::Pin;
use futures::{
    io::AsyncWrite,
    task::{Context, Poll},
};
use pin_project::pin_project;
use std::io::Error as IoError;

#[pin_project]
#[derive(Debug)]
pub struct HashBuffer<B> {
    #[pin]
    buffer: B,
    hasher: Hasher,
}

impl<B> HashBuffer<B> {
    pub fn into_inner(self) -> (Hasher, B) {
        (self.hasher, self.buffer)
    }
    pub fn hasher(&self) -> &Hasher {
        &self.hasher
    }
    pub fn hash(&mut self) -> Hash {
        self.hasher.finalize()
    }
}

impl<B> HashBuffer<B> {
    pub fn new(buffer: B) -> Self {
        Self {
            buffer,
            hasher: Hasher::new(),
        }
    }
}

impl<B> AsyncWrite for HashBuffer<B>
where
    B: AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, IoError>> {
        let p = self.project();
        p.hasher.update(buf);
        p.buffer.poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        self.project().buffer.poll_flush(cx)
    }
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), IoError>> {
        self.project().buffer.poll_close(cx)
    }
}

#[pin_project]
#[derive(Debug)]
pub struct Content<R> {
    size: u64,
    #[pin]
    content: R,
}

impl<R> Content<R> {
    pub fn new(size: u64, content: R) -> Self {
        Self { size, content }
    }

    pub fn len(&self) -> u64 {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn into_inner(self) -> (u64, R) {
        (self.size, self.content)
    }
}

impl<R> futures::io::AsyncRead for Content<R>
where
    R: futures::io::AsyncRead,
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.project();
        this.content.poll_read(cx, buf)
    }

    fn poll_read_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &mut [std::io::IoSliceMut<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let this = self.project();
        this.content.poll_read_vectored(cx, bufs)
    }
}
