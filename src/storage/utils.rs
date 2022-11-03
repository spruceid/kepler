use aws_types::sdk_config::SdkConfig;
use core::pin::Pin;
use futures::{
    executor::block_on,
    io::{AllowStdIo, AsyncWrite, Error},
    task::{Context, Poll},
};
use libipld::cid::multihash::Hasher;
use pin_project::pin_project;

pub fn aws_config() -> SdkConfig {
    block_on(async { aws_config::from_env().load().await })
}

#[pin_project]
#[derive(Debug, Clone)]
pub struct HashBuffer<H, B> {
    #[pin]
    buffer: B,
    hasher: H,
}

impl<H, B> HashBuffer<H, B> {
    pub fn into_inner(self) -> (H, B) {
        (self.hasher, self.buffer)
    }
}

impl<H, B> HashBuffer<H, B>
where
    H: Default,
{
    pub fn new(buffer: B) -> Self {
        Self {
            buffer,
            hasher: H::default(),
        }
    }
}

impl<H, B> AsyncWrite for HashBuffer<H, B>
where
    H: Hasher,
    B: AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        let p = self.project();
        p.hasher.update(buf);
        p.buffer.poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().buffer.poll_flush(cx)
    }
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.project().buffer.poll_close(cx)
    }
}
