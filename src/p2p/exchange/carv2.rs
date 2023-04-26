use futures::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    TryStream,
};
use libipld::Cid;
use std::io::Error as IoError;

use super::carv1::{write as v1_write, DataSection, Header as V1Header, WriteError};
use crate::storage::ImmutableStore;

const CAR_V2_PRAGMA: [u8; 11] = [
    0x0a, 0xa1, 0x67, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x02,
];

const HEADER_LEN: u64 = 40;

pub struct Characteristics([u8; 16]);

impl Characteristics {
    pub fn new() -> Self {
        Characteristics([0; 16])
    }

    pub fn fully_indexed(&mut self, indexed: bool) -> &mut Self {
        let byte_0 = self.0[0];
        self.0[0] = if indexed {
            byte_0 | 0b1000_0000
        } else {
            byte_0 & 0b0111_1111
        };
        self
    }
}

impl AsRef<[u8]> for Characteristics {
    fn as_ref(&self) -> &[u8] {
        &self.0.as_ref()
    }
}

pub struct Header {
    characteristics: Characteristics,
    data_offset: u64,
    data_size: u64,
    index_offset: u64,
}

impl Header {
    pub fn new() -> Self {
        Header {
            characteristics: Characteristics::new(),
            data_offset: 0,
            data_size: 0,
            index_offset: 0,
        }
    }

    pub async fn write_to<W>(&self, writer: &mut W) -> Result<(), IoError>
    where
        W: AsyncWrite + Unpin,
    {
        writer.write_all(self.characteristics.as_ref()).await?;
        writer.write_all(&self.data_offset.to_be_bytes()).await?;
        writer.write_all(&self.data_size.to_be_bytes()).await?;
        writer.write_all(&self.index_offset.to_be_bytes()).await?;
        Ok(())
    }
}

pub async fn write<W, S, R, E>(
    header: &Header,
    v1_header: &V1Header,
    data: &mut S,
    index: Option<()>,
    writer: &mut W,
) -> Result<(), WriteError<S::Error>>
where
    W: AsyncWrite + Unpin,
    S: TryStream<Ok = DataSection<R>, Error = E> + Unpin,
    R: AsyncRead,
{
    writer.write_all(&CAR_V2_PRAGMA).await?;
    header.write_to(writer).await?;

    v1_write(v1_header, data, writer).await?;

    // TODO write index if present

    Ok(())
}
