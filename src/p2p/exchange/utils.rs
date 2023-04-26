use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libipld::cid::{
    multihash::{Code, MultihashDigest},
    Cid, Error as CidError,
};
use std::io::Error as IoError;
use unsigned_varint::aio;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error(transparent)]
    Cid(#[from] CidError),
    #[error(transparent)]
    VarInt(#[from] unsigned_varint::decode::Error),
}

impl From<unsigned_varint::io::ReadError> for Error {
    fn from(e: unsigned_varint::io::ReadError) -> Self {
        match e {
            unsigned_varint::io::ReadError::Io(e) => e.into(),
            unsigned_varint::io::ReadError::Decode(e) => e.into(),
        }
    }
}

/// incremental parsing of a CID from an AsyncRead
pub async fn read_cid<R>(mut reader: R) -> Result<Cid, Error>
where
    R: AsyncRead + Unpin,
{
    use Code::*;
    let mut buf = [0u8; 1];
    reader.read_exact(&mut buf).await?;
    match buf[0] {
        // CID v0, should never really happen
        0x12 => {
            let mut buf = [0u8; 32];
            reader.read_exact(&mut buf).await?;
            Ok(Cid::new_v0(Code::Sha2_256.digest(&buf))?)
        }
        // CID v1
        0x20 => {
            let codec = aio::read_u64(&mut reader).await?;
            let mh_code =
                Code::try_from(aio::read_u64(&mut reader).await?).map_err(CidError::from)?;
            let mh = match mh_code {
                Sha2_256 | Sha3_256 | Keccak256 | Blake2b256 | Blake2s256 | Blake3_256 => {
                    let mut buf = [0u8; 32];
                    reader.read_exact(&mut buf).await?;
                    mh_code.wrap(&buf)
                }
                Sha2_512 | Sha3_512 | Keccak512 | Blake2b512 => {
                    let mut buf = [0u8; 64];
                    reader.read_exact(&mut buf).await?;
                    mh_code.wrap(&buf)
                }
                Sha3_224 | Keccak224 => {
                    let mut buf = [0u8; 28];
                    reader.read_exact(&mut buf).await?;
                    mh_code.wrap(&buf)
                }
                Sha3_384 | Keccak384 => {
                    let mut buf = [0u8; 48];
                    reader.read_exact(&mut buf).await?;
                    mh_code.wrap(&buf)
                }
                Blake2s128 => {
                    let mut buf = [0u8; 16];
                    reader.read_exact(&mut buf).await?;
                    mh_code.wrap(&buf)
                }
            }
            .map_err(CidError::from)?;
            Ok(Cid::new_v1(codec, mh))
        }
        _ => Err(Error::Cid(CidError::InvalidCidVersion)),
    }
}

pub struct Leb128Reader(u64, u8);

impl Leb128Reader {
    pub fn new() -> Self {
        Self(0, 0)
    }

    pub fn read(&mut self, byte: u8) -> Option<u64> {
        self.0 |= ((byte & 0x7f) as u64) << self.1;
        self.1 += 7;
        if byte & 0x80 == 0 {
            return Some(self.0);
        }
        None
    }
}

pub async fn read_leb128<R>(mut reader: R) -> Result<u64, IoError>
where
    R: AsyncRead + Unpin,
{
    let mut buf = [0u8; 1];
    let mut result = 0u64;
    let mut shift = 0u8;
    loop {
        reader.read_exact(&mut buf).await?;
        let byte = buf[0];
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
    }
}

pub async fn write_leb128<W>(value: u64, mut writer: W) -> Result<usize, IoError>
where
    W: AsyncWrite + Unpin,
{
    let mut buf = [0u8; 1];
    let mut written = 0;
    let mut value = value;
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf[0] = byte;
        writer.write_all(&buf).await?;
        written += 1;
        if value == 0 {
            return Ok(written);
        }
    }
}
