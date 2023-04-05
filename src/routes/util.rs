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
    limit: u64,
    written: u64,
}

impl<R> LimitedReader<R> {
    pub fn new(inner: R, limit: u64) -> Self {
        Self {
            inner,
            limit,
            written: 0,
        }
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    pub fn written(&self) -> u64 {
        self.written
    }
}

#[derive(thiserror::Error, Debug)]
#[error("limit exceeded")]
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
        // calculate the number of bytes that can be read
        let max_remaining = if let Some(remaining) = (*this.limit).checked_sub(*this.written) {
            // it's ok if remaining is 0 here, as writing 0 bytes won't change anything
            remaining
        } else {
            // limit already exceeded somehow
            return Poll::Ready(Err(IoError::new(ErrorKind::Other, LimitExceeded)));
        };

        match this.inner.poll_read(cx, buf) {
            Poll::Ready(Ok(n)) if n as u64 > max_remaining => {
                Poll::Ready(Err(IoError::new(ErrorKind::Other, LimitExceeded)))
            }
            Poll::Ready(Ok(n)) => {
                *this.written += n as u64;
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
