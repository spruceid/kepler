use libp2p::{identify::Config as OIdentifyConfig, identity::PublicKey};
pub mod behaviour;
pub mod relay;
pub mod transport;

const PROTOCOL_VERSION: &'static str = "kepler/0.1.0";

pub use builder::IdentifyConfig;

mod builder {
    use core::time::Duration;
    use derive_builder::Builder;
    use libp2p::{identify::Config as OIdentifyConfig, identity::PublicKey};
    #[derive(Builder, Default, Debug, Clone)]
    #[builder(build_fn(skip), setter(into), name = "IdentifyConfig", derive(Debug))]
    pub struct IdentifyConfigDummy {
        #[builder(field(type = "String"), default = "PROTOCOL_VERSION.into()")]
        protocol_version: String,
        #[builder(field(type = "Duration"), default = "Duration::from_millis(500)")]
        initial_delay: Duration,
        #[builder(field(type = "Duration"), default = "Duration::from_secs(300)")]
        interval: Duration,
        #[builder(field(type = "bool"), default = "false")]
        push_listen_addr_updates: bool,
        #[builder(field(type = "usize"), default = "0")]
        cache_size: usize,
    }
    pub fn to_config(config: IdentifyConfig, key: PublicKey) -> OIdentifyConfig {
        OIdentifyConfig::new(config.protocol_version, key)
            .with_initial_delay(config.initial_delay)
            .with_interval(config.interval)
            .with_push_listen_addr_updates(config.push_listen_addr_updates)
            .with_cache_size(config.cache_size)
    }
    pub fn convert(c: OIdentifyConfig) -> IdentifyConfig {
        IdentifyConfig {
            protocol_version: c.protocol_version,
            initial_delay: c.initial_delay,
            interval: c.interval,
            push_listen_addr_updates: c.push_listen_addr_updates,
            cache_size: c.cache_size,
        }
    }
}

impl IdentifyConfig {
    fn to_config(self, key: PublicKey) -> OIdentifyConfig {
        builder::to_config(self, key)
    }
}

impl From<OIdentifyConfig> for IdentifyConfig {
    fn from(c: OIdentifyConfig) -> Self {
        builder::convert(c)
    }
}
