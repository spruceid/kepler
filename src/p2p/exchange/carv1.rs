use super::utils::{read_cid, read_leb128, write_leb128, Error as CidReadError, Leb128Reader};
use async_stream::try_stream;
use futures::{
    channel::oneshot,
    io::{copy, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Take},
    stream::{Stream, TryStream, TryStreamExt},
};
use libipld::Cid;
use pin_project::pin_project;
use serde::{Deserialize, Serialize};
use std::{
    future::Future,
    io::{Error as IoError, ErrorKind},
    pin::Pin,
    task::Poll,
};

use crate::storage::Content;

#[derive(Serialize, Deserialize)]
pub struct Header {
    version: u8,
    roots: Vec<Cid>,
}

impl Header {
    pub async fn write_to<W>(&self, writer: &mut W) -> Result<(), IoError>
    where
        W: AsyncWrite + Unpin,
    {
        // TODO write serde_ipld_cbor
        Ok(())
    }

    pub async fn read_from<R>(reader: &mut R) -> Result<Self, IoError>
    where
        R: AsyncRead + Unpin,
    {
        // TODO write serde_ipld_cbor
        Ok(Self {
            version: 0,
            roots: Vec::new(),
        })
    }
}

pub struct DataSection<R> {
    cid: Cid,
    block: Content<R>,
}

impl<R> DataSection<R> {
    pub fn new(cid: Cid, block: Content<R>) -> Self {
        Self { cid, block }
    }
}

impl<R> DataSection<R>
where
    R: AsyncRead,
{
    pub async fn write_to<W>(self, writer: &mut W) -> Result<(), IoError>
    where
        W: AsyncWrite + Unpin,
    {
        let cid_bytes = self.cid.to_bytes();
        let total_len = cid_bytes.len() as u64 + self.block.len();
        write_leb128(total_len, writer).await?;
        writer.write_all(&cid_bytes).await?;
        copy(self.block, writer).await?;
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum WriteError<E> {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    Data(E),
}

pub async fn write<W, S, R, E>(
    header: &Header,
    data: &mut S,
    writer: &mut W,
) -> Result<(), WriteError<E>>
where
    W: AsyncWrite + Unpin,
    S: TryStream<Ok = DataSection<R>, Error = E> + Unpin,
    R: AsyncRead,
{
    header.write_to(writer).await?;

    while let Some(data) = data.try_next().await.map_err(WriteError::Data)? {
        data.write_to(writer).await?;
    }

    Ok(())
}

#[derive(thiserror::Error, Debug)]
pub enum ReadError {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    Cid(#[from] libipld::cid::Error),
    #[error(transparent)]
    Canceled(#[from] oneshot::Canceled),
}

impl From<CidReadError> for ReadError {
    fn from(e: CidReadError) -> Self {
        match e {
            CidReadError::Io(e) => Self::Io(e),
            CidReadError::Cid(e) => Self::Cid(e),
        }
    }
}

pub struct CarBlockReader<R> {
    reader: DelimitedReader<R>,
}

enum DelimitedReaderState<R> {
    Waiting(Waiting<R>),
    Available(Available<R>),
}

#[pin_project]
struct Available<R> {
    reader: Option<R>,
    len: Leb128Reader,
}

struct Waiting<R> {
    receiver: oneshot::Receiver<R>,
}

impl<R> Available<R> {
    fn new(reader: R) -> Self {
        Self {
            reader: Some(reader),
            len: Leb128Reader::new(),
        }
    }
}

impl<R> Future for Available<R>
where
    R: AsyncRead + Unpin,
{
    type Output = Result<(Waiting<R>, TakenReader<R>), IoError>;

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let len = match this.reader {
            Some(r) => {
                let mut buf = [0u8; 1];
                loop {
                    match Pin::new(r).poll_read(cx, &mut buf)? {
                        Poll::Ready(0) => {
                            return Poll::Ready(Err(IoError::new(ErrorKind::Other, "reader empty")))
                        }
                        Poll::Ready(1) => {}
                        Poll::Pending => return Poll::Pending,
                    };
                    if let Some(len) = this.len.read(buf[0]) {
                        break len;
                    }
                }
            }
            None => {
                return Poll::Ready(Err(IoError::new(ErrorKind::Other, "reader already taken")))
            }
        };
        // create oneshot
        let (tx, rx) = oneshot::channel();
        // create a new reader
        let reader = TakenReader {
            reader: this
                .reader
                .take()
                .ok_or_else(|| IoError::new(ErrorKind::Other, "reader already taken"))?
                .take(len),
            origin: tx,
        };
        Poll::Ready(Ok((Waiting { receiver: rx }, reader)))
    }
}

pub struct TakenReader<R> {
    reader: Take<R>,
    origin: oneshot::Sender<R>,
}

impl<R> DelimitedReaderState<R> {
    fn new(reader: R) -> Self {
        Self::Available(Available::new(reader))
    }
}

#[pin_project]
struct DelimitedReader<R> {
    reader: DelimitedReaderState<R>,
}

impl<R> DelimitedReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: DelimitedReaderState::new(reader),
        }
    }
}

impl<R> Stream for DelimitedReader<R>
where
    R: AsyncRead + Unpin,
{
    type Item = Result<TakenReader<R>, ReadError>;
    fn poll_next(
        self: Pin<&mut Self>,
        context: &mut std::task::Context,
    ) -> Poll<Option<Self::Item>> {
        let p = self.project();
        match p.reader {
            DelimitedReaderState::Waiting(r) => {
                // wait for receiver to be ready
                let ar = match r.receiver.try_recv() {
                    Ok(Some(ar)) => ar,
                    Ok(None) => return Poll::Pending,
                    // if reciever is dropped, should this return None?
                    Err(e) => return Poll::Ready(Some(Err(e.into()))),
                };
                *p.reader = DelimitedReaderState::new(ar);
                self.poll_next(context)
            }
            DelimitedReaderState::Available(a) => {
                // read len
                let (w, t) = futures::ready!(Pin::new(a).poll(context))?;
                *p.reader = DelimitedReaderState::Waiting(w);
                Poll::Ready(Some(Ok(t)))
            }
        }
    }
}

impl<R> CarBlockReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: DelimitedReader::new(reader),
        }
    }
}

pub async fn read_delimited<R>(
    mut reader: R,
) -> Result<
    (
        TakenReader<R>,
        impl Future<Output = Result<R, oneshot::Canceled>>,
    ),
    ReadError,
>
where
    R: AsyncRead + Unpin,
{
    let len = read_leb128(&mut reader).await?;
    let (tx, rx) = oneshot::channel();
    let reader = TakenReader {
        reader: reader.take(len),
        origin: tx,
    };
    Ok((reader, rx))
}

pub async fn read<R>(
    mut reader: R,
) -> Result<
    (
        Header,
        impl TryStream<Ok = DataSection<TakenReader<R>>, Error = ReadError>,
        impl Future<Output = R>,
    ),
    ReadError,
>
where
    R: AsyncRead + Unpin,
{
    let header = Header::read_from(&mut reader).await?;

    let (sender, reciever) = oneshot::channel();

    let data = try_stream! {
        loop {
            let len = match read_leb128(&mut reader).await {
                Ok(l) => l,
                Err(e) if e.kind() == ErrorKind::Eof => {
                    sender.send(reader).await?;
                    break;
                },
                Err(e) => Err(e)?
            };
            let cid = read_cid(&mut reader).await?;

            // TODO use libipld 0.16 to get cid len easily
            let cid_len = cid.to_bytes().len() as u64;

            let block = reader.take(len - cid_len);
            yield Ok((cid, block))
        }
    };

    Ok((header, data, reciever))
}
