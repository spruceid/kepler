use crate::storage::either::EitherError;
use futures::io::{AsyncRead, AsyncWrite};
use libp2p::{
    core::{
        muxing::StreamMuxerBox,
        transport::{dummy::DummyTransport, Boxed, MemoryTransport, OrTransport, Transport},
        upgrade,
    },
    dns::{ResolverConfig, ResolverOpts, TokioDnsConfig as DnsTransport},
    identity::Keypair,
    mplex,
    noise::{NoiseAuthenticated, NoiseError},
    tcp::tokio::Transport as TcpTransport,
    wasm_ext::ExtTransport,
    websocket::{tls::Config as WsTlsConfig, WsConfig as WsTransport},
    yamux, PeerId,
};
use std::io::Error as IoError;

pub fn build_transport<T>(
    t: T,
    timeout: std::time::Duration,
    keypair: &Keypair,
) -> Result<Boxed<(PeerId, StreamMuxerBox)>, NoiseError>
where
    T: 'static + Transport + Send + Unpin,
    T::Output: 'static + AsyncRead + AsyncWrite + Unpin + Send,
    T::Dial: Send,
    T::Error: 'static + Send + Sync,
    T::ListenerUpgrade: Send,
{
    Ok(t.upgrade(upgrade::Version::V1)
        // TODO replace with AWAKE protcol (or similar)
        .authenticate(NoiseAuthenticated::xx(keypair)?)
        .multiplex(upgrade::SelectUpgrade::new(
            yamux::YamuxConfig::default(),
            mplex::MplexConfig::default(),
        ))
        .timeout(timeout)
        .boxed())
}

pub trait IntoTransport {
    type T: Transport;
    type Error: std::error::Error;
    fn into_transport(self) -> Result<Self::T, Self::Error>;
    fn and<O: IntoTransport>(self, other: O) -> Both<Self, O>
    where
        Self: Sized,
    {
        Both(self, other)
    }
}

pub use libp2p::tcp::Config as TcpConfig;
pub use libp2p::wasm_ext::ffi::Transport as ExtConfig;

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

#[derive(Clone, Debug, Default)]
pub enum DnsResolver {
    #[default]
    System,
    Custom(Box<CustomDnsResolver>),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CustomDnsResolver {
    conf: ResolverConfig,
    opts: ResolverOpts,
}

impl CustomDnsResolver {
    pub fn config(&mut self, i: impl Into<ResolverConfig>) -> &mut Self {
        self.conf = i.into();
        self
    }
    pub fn options(&mut self, i: impl Into<ResolverOpts>) -> &mut Self {
        self.opts = i.into();
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct DnsConfig<B> {
    resolver: DnsResolver,
    base: B,
}

impl<B> DnsConfig<B> {
    pub fn new(i: impl Into<B>) -> Self {
        Self {
            base: i.into(),
            resolver: Default::default(),
        }
    }
    pub fn resolver(&mut self, i: impl Into<DnsResolver>) -> &mut Self {
        self.resolver = i.into();
        self
    }
    pub fn base(&mut self, i: impl Into<B>) -> &mut Self {
        self.base = i.into();
        self
    }
}

impl<B> IntoTransport for DnsConfig<B>
where
    B: IntoTransport,
    B::T: 'static + Send + Unpin,
    <B::T as Transport>::Output: 'static + AsyncRead + AsyncWrite + Send + Unpin,
    <B::T as Transport>::Dial: Send,
    <B::T as Transport>::Error: Send,
{
    type T = DnsTransport<B::T>;
    type Error = EitherError<IoError, B::Error>;
    fn into_transport(self) -> Result<Self::T, Self::Error> {
        match self.resolver {
            DnsResolver::System => {
                DnsTransport::system(self.base.into_transport().map_err(EitherError::B)?)
                    .map_err(EitherError::A)
            }
            DnsResolver::Custom(custom) => DnsTransport::custom(
                self.base.into_transport().map_err(EitherError::B)?,
                custom.conf,
                custom.opts,
            )
            .map_err(EitherError::A),
        }
    }
}

pub const WS_MAX_DATA_SIZE: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct WsConfig<T> {
    base: T,
    max_redirects: u8,
    max_data_size: usize,
    deflate: bool,
    tls: WsTlsConfig,
}

impl<T> WsConfig<T> {
    pub fn new(b: impl Into<T>) -> Self {
        Self {
            base: b.into(),
            max_redirects: 0,
            max_data_size: WS_MAX_DATA_SIZE,
            deflate: false,
            tls: WsTlsConfig::client(),
        }
    }
    pub fn base(&mut self, i: impl Into<T>) -> &mut Self {
        self.base = i.into();
        self
    }
    pub fn max_redirects(&mut self, i: impl Into<u8>) -> &mut Self {
        self.max_redirects = i.into();
        self
    }
    pub fn max_data_size(&mut self, i: impl Into<usize>) -> &mut Self {
        self.max_data_size = i.into();
        self
    }
    pub fn deflate(&mut self, i: impl Into<bool>) -> &mut Self {
        self.deflate = i.into();
        self
    }
    pub fn tls(&mut self, i: impl Into<WsTlsConfig>) -> &mut Self {
        self.tls = i.into();
        self
    }
}

impl<T> Default for WsConfig<T>
where
    T: Default,
{
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<B> IntoTransport for WsConfig<B>
where
    B: IntoTransport,
    B::T: 'static + Send + Unpin,
    <B::T as Transport>::Output: 'static + AsyncRead + AsyncWrite + Send + Unpin,
    <B::T as Transport>::Dial: Send,
    <B::T as Transport>::Error: Send,
    <B::T as Transport>::ListenerUpgrade: Send,
{
    type T = WsTransport<B::T>;
    type Error = B::Error;
    fn into_transport(self) -> Result<Self::T, Self::Error> {
        let mut ws = WsTransport::new(self.base.into_transport()?);
        ws.set_max_redirects(self.max_redirects)
            .set_max_data_size(self.max_data_size)
            .set_tls_config(self.tls)
            .use_deflate(self.deflate);
        Ok(ws)
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
