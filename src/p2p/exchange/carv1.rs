use futures::{
    io::{copy, AsyncRead, AsyncWrite, AsyncWriteExt},
    {TryStream, TryStreamExt},
};
use async_stream::try_stream;
use libipld::Cid;
use serde::{Deserialize, Serialize};
use std::io::Error as IoError;

use crate::storage::{Content, ImmutableStore};

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
        // TODO encode as varint
        writer.write_all(&total_len.to_be_bytes()).await?;
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

pub async fn read<R, E>(
    reader: &mut R,
) -> Result<(Header, impl TryStream<Ok = DataSection<Take<R>>, Error = E>), IoError>
where
    R: AsyncRead + Unpin,
{
    // TODO read header
    let header = Header {
        version: 0,
        roots: vec![],
    };
        if len == 0 {
            break;
        }

        let mut cid_bytes = vec![0u8; len as usize];
        reader.read_exact(&mut cid_bytes).await?;
        let cid = Cid::try_from(cid_bytes).map_err(|_| IoError::from(IoErrorKind::InvalidData))?;
        cids.push(cid);
    }

    Ok((header, cids))
}
