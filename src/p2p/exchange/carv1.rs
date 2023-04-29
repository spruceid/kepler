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

    pub fn into_inner(self) -> (Cid, Content<R>) {
        (self.cid, self.block)
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

pub async fn write_carv1<W, S, R, E>(
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

#[derive(Default)]
pub enum TakenReader<R> {
    #[default]
    Finished,
    Unfinished(Take<R>, Option<oneshot::Sender<R>>),
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
        match std::mem::take(self) {
            Self::Unfinished(r, Some(tx)) => {
                tx.send(r.into_inner()).map_err(|_| oneshot::Canceled)?;
            }
            _ => (),
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
        // boy I hope this doesnt need to be pinned in a better way
        match *self {
            TakenReader::Finished => Poll::Ready(Ok(0)),
            TakenReader::Unfinished(ref mut reader, _) => {
                let poll = Pin::new(reader).poll_read(cx, buf);
                if let Poll::Ready(Ok(0)) = poll {
                    let _ = self.finish();
                }
                poll
            }
        }
    }

    fn poll_read_vectored(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &mut [std::io::IoSliceMut<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match *self {
            TakenReader::Finished => Poll::Ready(Ok(0)),
            TakenReader::Unfinished(ref mut reader, _) => {
                let poll = Pin::new(reader).poll_read_vectored(cx, bufs);
                if let Poll::Ready(Ok(0)) = poll {
                    let _ = self.finish();
                }
                poll
            }
        }
    }
}

pub(crate) enum ReaderState<R, F> {
    Empty(R),
    Element(DataSection<TakenReader<R>>, F),
}

pub(crate) async fn read_section<R>(
    mut reader: R,
) -> Result<ReaderState<R, oneshot::Receiver<R>>, ReadError>
where
    R: AsyncRead + Unpin,
{
    // check if reader is already empty (if it can't read 1 byte, it's empty)
    let mut buf = [0u8; 1];
    match reader.read_exact(&mut buf).await {
        Ok(_) => (),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Ok(ReaderState::Empty(reader))
        }
        Err(e) => return Err(e.into()),
    };

    let len = read_leb128(buf.chain(&mut reader)).await?;
    let cid = read_cid(&mut reader).await?;
    let cid_len = cid.to_bytes().len() as u64;
    let block_len = len - cid_len;
    let (reader, rx) = TakenReader::new(reader, block_len);
    Ok(ReaderState::Element(
        DataSection::new(cid, Content::new(block_len, reader)),
        rx,
    ))
}

// this function should take a reader and stream out length-delimited cid-block pairs
// until the reader is empty. once the reader is empty, the returned future should
// resolve with the value of the reader
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
    // read the header
    let header = Header::read_from(&mut reader).await?;
    // setup the channel for completion state
    let (final_tx, final_rx) = oneshot::channel();
    let stream = try_stream! {
        loop {
            // try read a section
            match read_section(reader).await? {
                // section is empty, send reader to completion channel
                ReaderState::Empty(r) => {final_tx.send(r).map_err(|_| oneshot::Canceled)?; break},
                // reader is not empty, send section to stream
                ReaderState::Element(ds, rx) => {
                    yield ds;
                    reader = rx.await?;
                }
            }
        }
    };
    Ok((header, stream, final_rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use libipld::{multihash::Code, raw::RawCodec, Block, DefaultParams};

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

    #[test]
    async fn reader() {
        let header = Header {
            version: 1,
            roots: Vec::new(),
        };

        let (cid1, block1) =
            Block::<DefaultParams>::encode(RawCodec, Code::Sha3_256, &vec![0u8; 10])
                .expect("block encoding to work")
                .into_inner();

        let (cid2, block2) =
            Block::<DefaultParams>::encode(RawCodec, Code::Sha3_256, &vec![1u8; 24])
                .expect("block encoding to work")
                .into_inner();

        println!("{:?}", cid1);
        println!("{:?}", cid2);
        let cid_len = cid1.to_bytes().len() as u64;

        let mut buf = Vec::with_capacity(34);
        header
            .write_to(&mut buf)
            .await
            .expect("header write to work");

        write_leb128(block1.len() as u64 + cid_len, &mut buf)
            .await
            .expect("leb128 write to work");
        buf.extend_from_slice(cid1.to_bytes().as_slice());
        buf.extend_from_slice(&block1);

        write_leb128(block2.len() as u64 + cid_len, &mut buf)
            .await
            .expect("leb128 write to work");
        buf.extend_from_slice(cid2.to_bytes().as_slice());
        buf.extend_from_slice(&block2);

        let (read_header, stream, final_rx) = stream_carv1(buf.as_slice())
            .await
            .expect("stream from buffer to work");
        assert_eq!(read_header, header);

        let mut s = Box::pin(stream);

        let (rcid1, mut rblock1) = s
            .next()
            .await
            .expect("no read error")
            .expect("there should be a section")
            .into_inner();
        assert_eq!(rcid1, cid1);
        let mut buf1 = Vec::new();
        rblock1
            .read_to_end(&mut buf1)
            .await
            .expect("there should be a block");
        assert_eq!(buf1, block1);

        let (rcid2, mut rblock2) = s
            .next()
            .await
            .expect("no read error")
            .expect("there should be a section")
            .into_inner();
        assert_eq!(rcid2, cid2);
        let mut buf2 = Vec::new();
        rblock2
            .read_to_end(&mut buf2)
            .await
            .expect("there should be a block");
        assert_eq!(buf2, block2);

        assert_eq!(final_rx.await.unwrap(), buf);
    }
}
