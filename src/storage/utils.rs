use aws_types::sdk_config::SdkConfig;
use core::pin::Pin;
use futures::{
    executor::block_on,
    io::{copy, AsyncRead, AsyncWrite, AsyncWriteExt},
    task::{Context, Poll},
};
use libipld::cid::multihash::{
    Blake2b256, Blake2b512, Blake2s128, Blake2s256, Blake3_256, Code, Error as MultihashError,
    Hasher, Keccak224, Keccak256, Keccak384, Keccak512, Multihash, MultihashDigest, Sha2_256,
    Sha2_512, Sha3_224, Sha3_256, Sha3_384, Sha3_512,
};
use pin_project::pin_project;
use std::io::Error as IoError;

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

pub async fn copy_in<B>(
    data: impl AsyncRead,
    buffer: B,
    hash_type: Code,
) -> Result<(Multihash, B), MultihashError>
where
    B: AsyncWrite + Unpin,
{
    Ok(match hash_type {
        Code::Sha2_256 => {
            let mut hb = HashBuffer::<Sha2_256, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Sha2_512 => {
            let mut hb = HashBuffer::<Sha2_512, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Sha3_224 => {
            let mut hb = HashBuffer::<Sha3_224, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Sha3_256 => {
            let mut hb = HashBuffer::<Sha3_256, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Sha3_384 => {
            let mut hb = HashBuffer::<Sha3_384, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Sha3_512 => {
            let mut hb = HashBuffer::<Sha3_512, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Keccak224 => {
            let mut hb = HashBuffer::<Keccak224, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Keccak256 => {
            let mut hb = HashBuffer::<Keccak256, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Keccak384 => {
            let mut hb = HashBuffer::<Keccak384, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Keccak512 => {
            let mut hb = HashBuffer::<Keccak512, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Blake2b256 => {
            let mut hb = HashBuffer::<Blake2b256, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Blake2b512 => {
            let mut hb = HashBuffer::<Blake2b512, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Blake2s128 => {
            let mut hb = HashBuffer::<Blake2s128, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Blake2s256 => {
            let mut hb = HashBuffer::<Blake2s256, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        Code::Blake3_256 => {
            let mut hb = HashBuffer::<Blake3_256, B>::new(buffer);
            copy(data, &mut hb).await?;
            hb.flush().await?;
            let (mut h, b) = hb.into_inner();
            (hash_type.wrap(h.finalize())?, b)
        }
        c => return Err(MultihashError::UnsupportedCode(c.into())),
    })
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
