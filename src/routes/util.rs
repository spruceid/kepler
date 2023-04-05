use futures::io::AsyncRead;
use pin_project::pin_project;
use std::{
    io::{Error as IoError, ErrorKind},
    task::Poll,
};

/// LimitedRead wraps an AsyncRead and limits the number of bytes that can be read.
///
/// If the limit is exceeded, the read will return an error.
#[pin_project]
#[derive(Debug)]
pub struct LimitedReader<R> {
    #[pin]
    inner: R,
    remaining: u64,
}

impl<R> LimitedReader<R> {
    pub fn new(inner: R, limit: u64) -> Self {
        Self {
            inner,
            remaining: limit,
        }
    }

    pub fn remaining_limit(&self) -> u64 {
        self.remaining
    }
}

#[derive(thiserror::Error, Debug)]
#[error("This write will exceeded the storage limit")]
struct LimitExceeded;

impl<R> AsyncRead for LimitedReader<R>
where
    R: AsyncRead,
{
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context,
        buf: &mut [u8],
    ) -> Poll<Result<usize, IoError>> {
        let this = self.project();

        match this.inner.poll_read(cx, buf) {
            Poll::Ready(Ok(n)) if n as u64 > *this.remaining => {
                // TODO once io_error_more is stable, use ErrorKind::FileTooLarge
                Poll::Ready(Err(IoError::new(ErrorKind::Other, LimitExceeded)))
            }
            Poll::Ready(Ok(n)) => {
                // it's ok if remaining is 0 here, as writing 0 bytes won't change anything
                // also we checked n > remaining above so it shouldnt underflow
                *this.remaining -= n as u64;
                Poll::Ready(Ok(n))
            }
            r => r,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use futures::io::AsyncReadExt;

    #[test]
    async fn test_limit() {
        let data = b"hello world";
        let mut buf = Vec::with_capacity(data.len() + 10);

        // use a reader with limit above len
        let mut reader = LimitedReader::new(&data[..], data.len() as u64 + 1);

        let n = reader.read_to_end(&mut buf).await.unwrap();
        assert_eq!(n, data.len());

        // use a reader with limit equal to len
        let mut reader = LimitedReader::new(&data[..], data.len() as u64);
        let n = reader.read_to_end(&mut buf).await.unwrap();
        assert_eq!(n, data.len());

        // use a reader with limit below data len
        let mut reader = LimitedReader::new(&data[..], data.len() as u64 - 1);
        let r = reader.read_to_end(&mut buf).await;
        assert!(r.is_err());
    }
}
