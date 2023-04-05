use aws_types::sdk_config::SdkConfig;
use core::pin::Pin;
use futures::{
    executor::block_on,
    io::{copy, AsyncRead, AsyncWrite, AsyncWriteExt},
    task::{Context, Poll},
};
use kepler_lib::libipld::cid::multihash::{
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

macro_rules! write_with_multihash {
    ($data:ident, $buffer:ident, $code:ident, $($hashes:ident),*) => {
        match $code {
            $(Code::$hashes => {
                let mut hb = HashBuffer::<$hashes, B>::new($buffer);
                copy($data, &mut hb).await?;
                hb.flush().await?;
                let (mut h, b) = hb.into_inner();
                Ok((Code::$hashes.wrap(h.finalize())?, b))
            },)*
            _ => Err(MultihashError::UnsupportedCode($code.into())),
        }
    };
}

pub async fn copy_in<B>(
    data: impl AsyncRead,
    buffer: B,
    hash_type: Code,
) -> Result<(Multihash, B), MultihashError>
where
    B: AsyncWrite + Unpin,
{
    write_with_multihash!(
        data, buffer, hash_type, Sha2_256, Sha2_512, Sha3_224, Sha3_256, Sha3_384, Sha3_512,
        Keccak224, Keccak256, Keccak384, Keccak512, Blake2b256, Blake2b512, Blake2s128, Blake2s256,
        Blake3_256
    )
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
