use aws_types::sdk_config::SdkConfig;
use futures::executor::block_on;

pub fn aws_config() -> SdkConfig {
    block_on(async { aws_config::from_env().load().await })
}
