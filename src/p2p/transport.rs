use derive_builder::Builder;
use libp2p::{
    core::transport::{dummy::DummyTransport, MemoryTransport},
    dns::{ResolverConfig, ResolverOpts},
    identity::Keypair,
    mplex::MplexConfig,
    tcp::GenTcpConfig,
    wasm_ext::ffi,
    websocket::tls::Config as WsTlsConfig,
    yamux::YamuxConfig,
    relay::v2::client::{Client, transport::ClientTransport}
};
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum DnsResolver {
    System,
    Custom(DnsConfig),
}

impl Default for DnsResolver {
    fn default() -> Self {
        Self::System
    }
}

#[derive(Builder, Debug, Clone)]
pub struct DnsConfig {
    conf: ResolverConfig,
    opts: ResolverOpts,
}

pub const MAX_DATA_SIZE: usize = 256 * 1024 * 1024;

fn client() -> WsTlsConfig {
    WsTlsConfig::client()
}

#[derive(Builder, Debug, Clone)]
#[builder(
    build_fn(skip),
    setter(into),
    derive(Debug),
    name = "WsConfig",
    pattern = "owned"
)]
pub struct WsConfigDummy {
    #[builder(field(type = "u8"), default = "0")]
    max_redirects: u8,
    #[builder(field(type = "usize"), default = "MAX_DATA_SIZE")]
    max_data_size: usize,
    #[builder(field(type = "bool"), default = "false")]
    deflate: bool,
    #[builder(field(type = "WsTlsConfig"), default = "client()")]
    tls: WsTlsConfig,
}

#[derive(Builder)]
#[builder(
    build_fn(skip),
    setter(into),
    name = "TransportConfig",
    pattern = "owned"
)]
pub struct TransportConfigDummy {
    #[builder(field(type = "bool"), default = "false")]
    memory: bool,
    tcp: GenTcpConfig,
    external: ffi::Transport,
    dns: DnsResolver,
    websocket: WsConfig,
    #[builder(field(type = "bool"), default = "false")]
    relay: bool,
    mplex: MplexConfig,
    yamux: YamuxConfig,
    timeout_incoming: Duration,
    timeout_outgoing: Duration,
}

impl TransportConfig {
    fn build(self, keypair: Keypair) -> (Boxed<(PeerId, StreamMuxerBox)>, Option<Client>) {
        let d = match (&self.memory, &self.tcp, &self.external, &self.relay) {
            (false, None, None, false) => DummyTransport::new(),
            (true, None) => MemoryTransport::new(),

        }
    }
}
