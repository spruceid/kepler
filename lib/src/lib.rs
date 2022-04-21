pub mod resource;
pub mod zcap;

pub use didkit;
pub use ssi;

#[cfg(feature = "cacao_to_zcap")]
pub mod cacao_zcap {
    pub use ::cacao_zcap::translation::cacao_to_zcap::{cacao_to_zcap, CacaoToZcapError};
}

#[cfg(feature = "cacao_to_zcap")]
pub mod cacaos {
    pub use ::cacao_zcap::cacaos::{
        siwe_cacao::{SIWESignature, SignInWithEthereum},
        BasicSignature, CACAO,
    };
}

#[cfg(feature = "cacao_to_zcap")]
pub mod siwe {
    pub use ::cacao_zcap::cacaos::siwe::{nonce::generate_nonce, Message, TimeStamp, Version};
}
