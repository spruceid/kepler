use core::time::Duration;
use derive_builder::Builder;
use libp2p::{identify::Config as OIdentifyConfig, identity::PublicKey};
pub mod behaviour;

#[derive(Builder, Default, Debug, Clone)]
pub struct IdentifyConfig {
    #[builder(setter(into), default = "Duration::from_millis(500)")]
    initial_delay: Duration,
    #[builder(setter(into), default = "Duration::from_secs(300)")]
    interval: Duration,
    #[builder(setter(into), default = "false")]
    push_listen_addr_updates: bool,
    #[builder(setter(into), default = "0")]
    cache_size: usize,
}

const PROTOCOL_VERSION: &'static str = "kepler/0.1.0";

impl IdentifyConfig {
    fn to_config(self, key: PublicKey) -> OIdentifyConfig {
        OIdentifyConfig::new(PROTOCOL_VERSION.to_string(), key)
            .with_initial_delay(self.initial_delay)
            .with_interval(self.interval)
            .with_push_listen_addr_updates(self.push_listen_addr_updates)
            .with_cache_size(self.cache_size)
    }
}

impl From<OIdentifyConfig> for IdentifyConfig {
    fn from(c: OIdentifyConfig) -> Self {
        Self {
            initial_delay: c.initial_delay,
            interval: c.interval,
            push_listen_addr_updates: c.push_listen_addr_updates,
            cache_size: c.cache_size,
        }
    }
}
