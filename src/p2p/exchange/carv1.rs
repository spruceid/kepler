use super::utils::{read_cid, read_dag_cbor_cid, read_leb128, write_leb128, Error as CidReadError};
use async_stream::try_stream;
use futures::{
    channel::oneshot,
    io::{copy, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, Take},
    stream::{Stream, TryStream, TryStreamExt},
};
use libipld::Cid;
use pin_project::pin_project;
use serde::{Deserialize, Serialize};
use serde_ipld_dagcbor::{to_writer, DecodeError, EncodeError};
use std::{
    future::Future,
    io::{Error as IoError, ErrorKind},
    pin::Pin,
    task::Poll,
};

use crate::storage::Content;

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct Header {
    version: u8,
    roots: Vec<Cid>,
}

impl Header {
    const INITIAL_BYTES: [u8; 9] = [0xa2, 0x67, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e];
    const ROOTS_BYTES: [u8; 6] = [0x65, 0x72, 0x6f, 0x6f, 0x74, 0x73];
    pub async fn write_to<W>(&self, mut writer: W) -> Result<(), EncodeError<IoError>>
    where
        W: AsyncWrite + Unpin,
    {
        let mut buf = Vec::new();
        to_writer(&mut buf, self)?;
        writer.write_all(&mut buf).await?;
        Ok(())
    }

    // no async serde decoding :(, so we have to do it manually
    pub async fn read_from<R>(mut reader: R) -> Result<Self, DecodeError<IoError>>
    where
        R: AsyncRead + Unpin,
    {
        let mut buf = [0u8; 9];
        reader.read_exact(&mut buf).await?;
        // expect opening of cbor map with 2 elements, then "version" key
        if buf != Header::INITIAL_BYTES {
            return Err(DecodeError::Msg("invalid header".to_string()));
        };

        // read version first, version < roots in dag-cbor
        reader.read_exact(&mut buf[0..1]).await?;
        let version = buf[0];

        // read roots
        reader.read_exact(&mut buf[0..6]).await?;
        if buf[0..6] != Header::ROOTS_BYTES {
            return Err(DecodeError::Msg("invalid header".to_string()));
        };

        // array tag + array len
        reader.read_exact(&mut buf[0..1]).await?;
        let array_tag = buf[0] >> 5;
        if array_tag != 4 {
            return Err(DecodeError::Mismatch {
                expect_major: 4,
                byte: array_tag,
            });
        };

        // assuming for now we'll never have more than 7 roots :/
        let array_len = buf[0] & 0b00011111;

        let mut roots = Vec::with_capacity(array_len.into());
        // for array len 'n' read n cids
        for _ in 0..array_len {
            roots.push(read_dag_cbor_cid(&mut reader).await.map_err(|e| match e {
                CidReadError::Io(e) => DecodeError::Read(e),
                _ => DecodeError::Msg(e.to_string()),
            })?);
        }

        Ok(Self { version, roots })
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
    pub async fn write_to<W>(self, mut writer: W) -> Result<(), IoError>
    where
        W: AsyncWrite + Unpin,
    {
        let cid_bytes = self.cid.to_bytes();
        let total_len = cid_bytes.len() as u64 + self.block.len();
        write_leb128(total_len, &mut writer).await?;
        writer.write_all(&cid_bytes).await?;
        copy(self.block, &mut writer).await?;
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum WriteError<E> {
    #[error(transparent)]
    Header(#[from] EncodeError<IoError>),
    #[error(transparent)]
    Data(E),
}

impl<E> From<IoError> for WriteError<E> {
    fn from(e: IoError) -> Self {
        Self::Header(EncodeError::Write(e))
    }
}

pub async fn write<W, S, R, E>(
    header: &Header,
    data: &mut S,
    mut writer: W,
) -> Result<(), WriteError<E>>
where
    W: AsyncWrite + Unpin,
    S: TryStream<Ok = DataSection<R>, Error = E> + Unpin,
    R: AsyncRead,
{
    header.write_to(&mut writer).await?;

    while let Some(data) = data.try_next().await.map_err(WriteError::Data)? {
        data.write_to(&mut writer).await?
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
    #[error(transparent)]
    Header(#[from] DecodeError<IoError>),
}

impl From<CidReadError> for ReadError {
    fn from(e: CidReadError) -> Self {
        match e {
            CidReadError::Io(e) => Self::Io(e),
            CidReadError::Cid(e) => Self::Cid(e),
        }
    }
}

// pub struct CarBlockReader<R> {
//     reader: DelimitedReader<R>,
// }

// enum DelimitedReaderState<R> {
//     Waiting(Waiting<R>),
//     Available(Available<R>),
// }

// #[pin_project]
// struct Available<R> {
//     #[pin]
//     len: Leb128Reader<R>,
// }

// struct Waiting<R> {
//     receiver: oneshot::Receiver<R>,
// }

// impl<R> Available<R> {
//     fn new(reader: R) -> Self {
//         Self {
//             len: Leb128Reader::new(reader),
//         }
//     }
// }

// impl<R> Future for Available<R>
// where
//     R: AsyncRead + Unpin,
// {
//     type Output = Result<(Waiting<R>, TakenReader<R>), IoError>;

//     fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
//         let this = self.project();
//         let (len, reader) = futures::ready!(this.len.poll(cx))?;
//         // create a new reader
//         let (reader, rx) = TakenReader::new(reader, len);
//         Poll::Ready(Ok((Waiting { receiver: rx }, reader)))
//     }
// }

#[pin_project(project = TakenReaderProj)]
pub enum TakenReader<R> {
    Finished,
    Unfinished(#[pin] Take<R>, Option<oneshot::Sender<R>>),
}

impl<R> TakenReader<R>
where
    R: AsyncRead,
{
    pub fn new(reader: R, limit: u64) -> (Self, oneshot::Receiver<R>) {
        let (tx, rx) = oneshot::channel();
        let reader = reader.take(limit);
        (Self::Unfinished(reader, Some(tx)), rx)
    }

    fn finish(&mut self) -> Result<(), oneshot::Canceled> {
        match self {
            Self::Finished => (),
            Self::Unfinished(r, tx) => {
                let sender = tx.take();
                match sender {
                    Some(sender) => sender.send(r.into_inner()).map_err(|_| oneshot::Canceled)?,
                    None => (),
                };
                *self = Self::Finished;
            }
        };
        Ok(())
    }
}

impl<R> AsyncRead for TakenReader<R>
where
    R: AsyncRead + Unpin,
{
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        {
            let mut this = self.project();
            match this {
                TakenReaderProj::Finished => return Poll::Ready(Ok(0)),
                TakenReaderProj::Unfinished(ref mut reader, _) => {
                    match Pin::new(reader).poll_read(cx, buf) {
                        Poll::Ready(Ok(0)) => true,
                        p => return p,
                    }
                }
            }
        };
        // we can only get here if we're finished but haven't sent yet
        let _ = self.finish();
        Poll::Ready(Ok(0))
    }

    fn poll_read_vectored(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &mut [std::io::IoSliceMut<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        {
            let mut this = self.project();
            match this {
                TakenReaderProj::Finished => return Poll::Ready(Ok(0)),
                TakenReaderProj::Unfinished(ref mut reader, _) => {
                    match reader.poll_read_vectored(cx, bufs) {
                        Poll::Ready(Ok(0)) => (),
                        p => return p,
                    }
                }
            }
        };
        // we can only get here if we're finished but haven't sent yet
        let _ = self.finish();
        Poll::Ready(Ok(0))
    }
}

// impl<R> DelimitedReaderState<R> {
//     fn new(reader: R) -> Self {
//         Self::Available(Available::new(reader))
//     }
// }

// #[pin_project]
// struct DelimitedReader<R> {
//     reader: DelimitedReaderState<R>,
// }

// impl<R> DelimitedReader<R> {
//     pub fn new(reader: R) -> Self {
//         Self {
//             reader: DelimitedReaderState::new(reader),
//         }
//     }
// }

// impl<R> Stream for DelimitedReader<R>
// where
//     R: AsyncRead + Unpin,
// {
//     type Item = Result<TakenReader<R>, ReadError>;
//     fn poll_next(
//         self: Pin<&mut Self>,
//         context: &mut std::task::Context,
//     ) -> Poll<Option<Self::Item>> {
//         let p = self.project();
//         match p.reader {
//             DelimitedReaderState::Waiting(r) => {
//                 // wait for receiver to be ready
//                 let ar = match r.receiver.try_recv() {
//                     Ok(Some(ar)) => ar,
//                     Ok(None) => return Poll::Pending,
//                     // if sender is dropped, should this return None?
//                     Err(e) => return Poll::Ready(Some(Err(e.into()))),
//                 };
//                 *p.reader = DelimitedReaderState::new(ar);
//                 // self.poll_next(context)
//                 Poll::Pending
//             }
//             DelimitedReaderState::Available(a) => {
//                 // read len
//                 let (w, t) = futures::ready!(Pin::new(a).poll(context))?;
//                 *p.reader = DelimitedReaderState::Waiting(w);
//                 Poll::Ready(Some(Ok(t)))
//             }
//         }
//     }
// }

// impl<R> CarBlockReader<R> {
//     pub fn new(reader: R) -> Self {
//         Self {
//             reader: DelimitedReader::new(reader),
//         }
//     }
// }

enum ReaderState<R, F> {
    Empty(R),
    Error(ReadError),
    Element(DataSection<TakenReader<R>>, F),
}

pub(crate) async fn read_section<R>(
    mut reader: R,
) -> Result<ReaderState<R, oneshot::Receiver<R>>, ReadError>
where
    R: AsyncRead + Unpin,
{
    let len = match read_leb128(&mut reader).await {
        Ok(len) => len,
        Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(ReaderState::Empty(reader)),
        Err(e) => return Err(ReadError::Io(e)),
    };
    let cid = read_cid(&mut reader).await?;
    let cid_len = cid.to_bytes().len() as u64;
    let block_len = len - cid_len;
    let (reader, rx) = TakenReader::new(reader, block_len);
    Ok(ReaderState::Element(
        DataSection::new(cid, Content::new(block_len, reader)),
        rx,
    ))
}

pub async fn stream_carv1<R>(
    mut reader: R,
) -> Result<
    (
        Header,
        impl Stream<Item = Result<DataSection<TakenReader<R>>, ReadError>>,
        impl Future<Output = Result<R, oneshot::Canceled>>,
    ),
    ReadError,
>
where
    R: AsyncRead + Unpin,
{
    let header = Header::read_from(&mut reader).await?;
    let (final_tx, final_rx) = oneshot::channel();
    let stream = try_stream! {
        while match read_section(reader).await? {
            ReaderState::Empty(r) => {final_tx.send(r).map_err(|_| oneshot::Canceled)?; false},
            ReaderState::Error(e) => {yield Err(e); false},
            ReaderState::Element(ds, rx) => {
                yield ds;
                reader = rx.await?;
                true
            }
        } {}
    };
    Ok((header, stream, final_rx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    async fn header() {
        let header = Header {
            version: 1,
            roots: vec![
                "bagaaierasords4njcts6vs7qvdjfcvgnume4hqohf65zsfguprqphs3icwea"
                    .parse()
                    .unwrap(),
            ],
        };
        let mut buf = Vec::new();
        header.write_to(&mut buf).await.unwrap();
        println!("{:x?}", buf);
        let deser = Header::read_from(&mut buf.as_slice()).await.unwrap();
        assert_eq!(header, deser);
    }
}
