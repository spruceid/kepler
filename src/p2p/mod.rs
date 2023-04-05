use core::time::Duration;

use libp2p::{identify::Config as OIdentifyConfig, identity::PublicKey};

pub mod behaviour;
pub mod relay;
pub mod transport;

pub const PROTOCOL_VERSION: &'static str = "kepler/0.1.0";

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct IdentifyConfig {
    protocol_version: String,
    initial_delay: Duration,
    interval: Duration,
    push_listen_addr_updates: bool,
    cache_size: usize,
}

impl IdentifyConfig {
    pub fn protocol_version(&mut self, i: impl Into<String>) -> &mut Self {
        self.protocol_version = i.into();
        self
    }
    pub fn initial_delay(&mut self, i: impl Into<Duration>) -> &mut Self {
        self.initial_delay = i.into();
        self
    }
    pub fn interval(&mut self, i: impl Into<Duration>) -> &mut Self {
        self.interval = i.into();
        self
    }
    pub fn push_listen_addr_updates(&mut self, i: impl Into<bool>) -> &mut Self {
        self.push_listen_addr_updates = i.into();
        self
    }
    pub fn cache_size(&mut self, i: impl Into<usize>) -> &mut Self {
        self.cache_size = i.into();
        self
    }
}

impl Default for IdentifyConfig {
    fn default() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.into(),
            initial_delay: Duration::from_millis(500),
            interval: Duration::from_secs(300),
            push_listen_addr_updates: false,
            cache_size: 0,
        }
    }
}

impl IdentifyConfig {
    fn to_config(self, key: PublicKey) -> OIdentifyConfig {
        OIdentifyConfig::new(self.protocol_version, key)
            .with_initial_delay(self.initial_delay)
            .with_interval(self.interval)
            .with_push_listen_addr_updates(self.push_listen_addr_updates)
            .with_cache_size(self.cache_size)
    }
}

impl From<OIdentifyConfig> for IdentifyConfig {
    fn from(c: OIdentifyConfig) -> Self {
        IdentifyConfig {
            protocol_version: c.protocol_version,
            initial_delay: c.initial_delay,
            interval: c.interval,
            push_listen_addr_updates: c.push_listen_addr_updates,
            cache_size: c.cache_size,
        }
    }
}
