use crate::storage::either::EitherError;
use derive_builder::Builder;
use futures::io::{AsyncRead, AsyncWrite};
use libp2p::{
    core::transport::{dummy::DummyTransport, MemoryTransport, OrTransport, Transport},
    dns::{ResolverConfig, ResolverOpts, TokioDnsConfig as DnsTransport},
    tcp::TokioTcpTransport as TcpTransport,
    wasm_ext::ExtTransport,
    websocket::{tls::Config as WsTlsConfig, WsConfig as WsTransport},
};
use std::io::Error as IoError;

pub trait IntoTransport {
    type T: Transport;
    type Error;
    fn into_transport(self) -> Result<Self::T, Self::Error>;
    fn and<O: IntoTransport>(self, other: O) -> Both<Self, O>
    where
        Self: Sized,
    {
        Both(self, other)
    }
}

pub use dns::{CustomDnsResolver, DnsConfig};
pub use libp2p::tcp::GenTcpConfig as TcpConfig;
pub use libp2p::wasm_ext::ffi::Transport as ExtConfig;
pub use ws::{WsConfig, WS_MAX_DATA_SIZE};

#[derive(Default, Debug, Clone, Hash, PartialEq, Eq)]
pub struct MemoryConfig;

#[derive(Default, Debug, Clone, Hash, PartialEq, Eq)]
pub struct DummyConfig;

#[derive(Default, Debug, Clone, Hash, PartialEq, Eq)]
pub struct Both<A, B>(pub A, pub B);

impl<A, B> IntoTransport for Both<A, B>
where
    A: IntoTransport,
    B: IntoTransport,
{
    type T = OrTransport<A::T, B::T>;
    type Error = EitherError<A::Error, B::Error>;
    fn into_transport(self) -> Result<Self::T, Self::Error> {
        Ok(OrTransport::new(
            self.0.into_transport().map_err(EitherError::A)?,
            self.1.into_transport().map_err(EitherError::B)?,
        ))
    }
}

#[derive(Clone, Debug)]
pub enum DnsResolver {
    System,
    Custom(CustomDnsResolver),
}

impl Default for DnsResolver {
    fn default() -> Self {
        Self::System
    }
}

mod dns {
    use super::*;
    #[derive(Builder)]
    #[builder(
        build_fn(skip),
        setter(into),
        derive(Debug),
        name = "CustomDnsResolver"
    )]
    pub struct CustomDnsResolverDummy {
        #[builder(field(type = "ResolverConfig"))]
        conf: ResolverConfig,
        #[builder(field(type = "ResolverOpts"))]
        opts: ResolverOpts,
    }

    #[derive(Builder)]
    #[builder(build_fn(skip), setter(into), derive(Debug), name = "DnsConfig")]
    pub struct DnsConfigDummy<B>
    where
        B: Default,
    {
        #[builder(field(type = "DnsResolver"))]
        resolver: DnsResolver,
        #[builder(field(type = "B"))]
        base: B,
    }

    pub fn convert<B>(c: DnsConfig<B>) -> Result<DnsTransport<B::T>, EitherError<IoError, B::Error>>
    where
        B: Default + IntoTransport,
        B::T: 'static + Send + Unpin,
        <B::T as Transport>::Output: 'static + AsyncRead + AsyncWrite + Send + Unpin,
        <B::T as Transport>::Dial: Send,
        <B::T as Transport>::Error: Send,
    {
        match c.resolver {
            DnsResolver::System => {
                DnsTransport::system(c.base.into_transport().map_err(EitherError::B)?)
                    .map_err(EitherError::A)
            }
            DnsResolver::Custom(custom) => DnsTransport::custom(
                c.base.into_transport().map_err(EitherError::B)?,
                custom.conf,
                custom.opts,
            )
            .map_err(EitherError::A),
        }
    }
}

impl<B> IntoTransport for DnsConfig<B>
where
    B: Default + IntoTransport,
    B::T: 'static + Send + Unpin,
    <B::T as Transport>::Output: 'static + AsyncRead + AsyncWrite + Send + Unpin,
    <B::T as Transport>::Dial: Send,
    <B::T as Transport>::Error: Send,
{
    type T = DnsTransport<B::T>;
    type Error = EitherError<IoError, B::Error>;
    fn into_transport(self) -> Result<Self::T, Self::Error> {
        dns::convert(self)
    }
}

mod ws {
    use super::*;
    pub const WS_MAX_DATA_SIZE: usize = 256 * 1024 * 1024;

    fn client() -> WsTlsConfig {
        WsTlsConfig::client()
    }

    #[derive(Builder)]
    #[builder(build_fn(skip), setter(into), derive(Debug), name = "WsConfig")]
    pub struct WsConfigDummy<T>
    where
        T: Default,
    {
        #[builder(field(type = "T"))]
        base: T,
        #[builder(field(type = "u8"), default = "0")]
        max_redirects: u8,
        #[builder(field(type = "usize"), default = "WS_MAX_DATA_SIZE")]
        max_data_size: usize,
        #[builder(field(type = "bool"), default = "false")]
        deflate: bool,
        // TODO this is cause some kind of error cos it has no Default
        // #[builder(field(type = "WsTlsConfig"), default = "client()")]
        // tls: WsTlsConfig,
    }

    pub fn convert<B>(c: WsConfig<B>) -> Result<WsTransport<B::T>, B::Error>
    where
        B: Default + IntoTransport,
        B::T: 'static + Send + Unpin,
        <B::T as Transport>::Output: 'static + AsyncRead + AsyncWrite + Send + Unpin,
        <B::T as Transport>::Dial: Send,
        <B::T as Transport>::Error: Send,
        <B::T as Transport>::ListenerUpgrade: Send,
    {
        let mut ws = WsTransport::new(c.base.into_transport()?);
        ws.set_max_redirects(c.max_redirects)
            .set_max_data_size(c.max_data_size)
            // .set_tls_config(c.tls)
            .use_deflate(c.deflate);
        Ok(ws)
    }
}

impl<B> IntoTransport for WsConfig<B>
where
    B: Default + IntoTransport,
    B::T: 'static + Send + Unpin,
    <B::T as Transport>::Output: 'static + AsyncRead + AsyncWrite + Send + Unpin,
    <B::T as Transport>::Dial: Send,
    <B::T as Transport>::Error: Send,
    <B::T as Transport>::ListenerUpgrade: Send,
{
    type T = WsTransport<B::T>;
    type Error = B::Error;
    fn into_transport(self) -> Result<Self::T, Self::Error> {
        ws::convert(self)
    }
}

impl IntoTransport for ExtConfig {
    type T = ExtTransport;
    type Error = std::convert::Infallible;
    fn into_transport(self) -> Result<Self::T, Self::Error> {
        Ok(Self::T::new(self))
    }
}

impl IntoTransport for MemoryConfig {
    type T = MemoryTransport;
    type Error = std::convert::Infallible;
    fn into_transport(self) -> Result<Self::T, Self::Error> {
        Ok(Self::T::new())
    }
}

impl IntoTransport for TcpConfig {
    type T = TcpTransport;
    type Error = std::convert::Infallible;
    fn into_transport(self) -> Result<Self::T, Self::Error> {
        Ok(Self::T::new(self))
    }
}

impl IntoTransport for DummyConfig {
    type T = DummyTransport;
    type Error = std::convert::Infallible;
    fn into_transport(self) -> Result<Self::T, Self::Error> {
        Ok(Self::T::new())
    }
}
