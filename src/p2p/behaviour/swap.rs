use crate::storage::ImmutableStore;
use core::task::Poll;
use exchange_protocol::RequestResponseCodec;
use futures::{
    io::{copy, AsyncRead, AsyncReadExt, AsyncWrite, Error as FIoError, Take},
    task::Context,
};
use libipld::Cid;
use libp2p::{
    autonat::Behaviour as AutoNat,
    dcutr::behaviour::Behaviour as Dcutr,
    gossipsub::Gossipsub,
    identify::Behaviour as Identify,
    kad::{
        record::store::{MemoryStore, RecordStore},
        Kademlia,
    },
    ping::Behaviour as Ping,
    relay::v2::client::Client,
    swarm::{
        behaviour::toggle::Toggle, NetworkBehaviour, NetworkBehaviourAction, PollParameters, Swarm,
    },
};
use std::io::Error as IoError;

#[derive(Clone, Debug)]
pub struct KeplerSwap;

pub struct SwapRequest {
    roots: Vec<Cid>,
    bloom: [u8; 256],
}

pub struct SwapResponse<R> {
    stream: R,
}

const ROOTLESS_CAR_V1_HEADER: [u8; 11] = [
    0x0a, 0xa1, 0x67, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x01,
];

impl<R> SwapResponse<R>
where
    R: AsyncRead + Unpin,
{
    pub async fn new(r: R) -> Result<Self, FIoError> {
        let mut header = [0u8; 11];
        r.read_exact(&mut header).await?;
        if header != ROOTLESS_CAR_V1_HEADER {
            return Err(todo!());
        };
        Ok(Self { stream: r })
    }
    pub async fn first(self) -> Option<Result<CarV1StreamBlock<R>, ()>> {
        CarV1StreamBlock::new(self.stream)
    }
}

pub struct CarV1StreamBlock<R> {
    cid: Cid,
    block: Take<R>,
}

impl<R> CarV1StreamBlock<R> {
    pub async fn write_and_next(self, store: impl ImmutableStore) -> Option<Result<Self, ()>> {
        store.write_keyed(&mut self.block, self.cid.hash()).await?;
        Self::new(self.block.into_inner()).await
    }
    pub async fn new(r: R) -> Option<Result<Self, ()>> {
        // read len
        // read cid
        // take r with len
        todo!()
    }
}

#[async_trait]
impl RequestResponseCodec for KeplerSwap {
    type Protocol = &'static str;
    type Request = SwapRequest;
    type Response<R> = SwapResponse<R> where R: AsyncRead + Send;

    async fn read_request<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: T,
    ) -> Result<Self::Request, IoError>
    where
        T: AsyncRead + Unpin + Send,
    {
        todo!()
    }

    async fn read_response<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: T,
    ) -> Result<Self::Response<T>, IoError>
    where
        T: AsyncRead + Unpin + Send,
    {
        SwapResponse::new(io).await
    }

    async fn write_request<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: T,
        req: Self::Request,
    ) -> Result<(), IoError>
    where
        T: AsyncWrite + Unpin + Send,
    {
        todo!()
    }

    async fn write_response<T, R>(
        &mut self,
        protocol: &Self::Protocol,
        io: T,
        res: Self::Response<R>,
    ) -> Result<(), IoError>
    where
        T: AsyncWrite + Unpin + Send,
        R: AsyncRead,
    {
        copy(res.stream, &mut io).await?;
        Ok(())
    }
}
