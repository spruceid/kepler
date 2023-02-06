use did_ethr::DIDEthr;
use did_method_key::DIDKey;
use did_onion::DIDOnion;
use did_pkh::DIDPKH;
use did_tz::DIDTz;
use did_web::DIDWeb;
use did_webkey::DIDWebKey;
use ssi::did::DIDMethods;
use std::env::VarError;

lazy_static::lazy_static! {
    pub static ref DID_METHODS: DIDMethods<'static> = {
        let mut methods = DIDMethods::default();
        methods.insert(Box::new(DIDKey));
        methods.insert(Box::<DIDTz>::default());
        methods.insert(Box::new(DIDEthr));
        // methods.insert(Box::new(DIDSol));
        methods.insert(Box::new(DIDWeb));
        methods.insert(Box::new(DIDWebKey));
        methods.insert(Box::new(DIDPKH));
        methods.insert(Box::new({
            let mut onion = DIDOnion::default();
            if let Some(url) = match std::env::var("DID_ONION_PROXY_URL") {
                Ok(url) => Some(url),
                Err(VarError::NotPresent) => None,
                Err(VarError::NotUnicode(err)) => {
                    eprintln!("Unable to parse DID_ONION_PROXY_URL: {err:?}");
                    None
                }
            } {
                onion.proxy_url = url;
            }
            onion
        }));
        methods
    };
}
