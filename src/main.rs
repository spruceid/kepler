#[macro_use]
extern crate rocket;
#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate tokio;

use anyhow::{anyhow, Error, Result};
use libipld::cid::{multibase::Base, Cid};
use rocket::{
    data::{Data, ToByteUnit},
    fairing::AdHoc,
    form::{DataField, Form, FromFormField},
    futures::stream::StreamExt,
    http::Header,
    response::{Debug, Stream as RocketStream},
    tokio::fs::read_dir,
    State,
};
// use rocket_cors::CorsOptions;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io::Cursor,
    path::{Path, PathBuf},
    str::FromStr,
};
use tokio::sync::{RwLock, RwLockReadGuard};
use tokio_stream::wrappers::ReadDirStream;
use tz::{TZAuth, TezosBasicAuthorization};

mod auth;
mod cas;
mod codec;
mod ipfs;
mod orbit;
mod tz;

use auth::{Action, AuthWrapper, AuthorizationToken};
use cas::ContentAddressedStorage;
use codec::{PutContent, SupportedCodecs};
use orbit::{create_orbit, Orbit, SimpleOrbit};

#[derive(PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CidWrap(Cid);

// Orphan rule requires a wrapper type for this :(
impl<'a> rocket::request::FromParam<'a> for CidWrap {
    type Error = anyhow::Error;
    fn from_param(param: &'a str) -> Result<CidWrap> {
        Ok(CidWrap(Cid::from_str(param)?))
    }
}

#[rocket::async_trait]
impl<'r> FromFormField<'r> for CidWrap {
    async fn from_data(field: DataField<'r, '_>) -> rocket::form::Result<'r, Self> {
        Ok(CidWrap(
            field
                .name
                .source()
                .parse()
                .map_err(|_| field.unexpected())?,
        ))
    }
}

struct Orbits<O>
where
    O: Orbit,
{
    pub stores: RwLock<BTreeMap<Cid, O>>,
    pub base_path: PathBuf,
}

#[rocket::async_trait]
pub trait OrbitCollection<O: Orbit> {
    async fn orbits(&self) -> RwLockReadGuard<BTreeMap<Cid, O>>;
    async fn add(&self, orbit: O) -> ();
}

impl<O: Orbit> Orbits<O> {
    fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            stores: RwLock::new(BTreeMap::new()),
            base_path: path.as_ref().to_path_buf(),
        }
    }
}

#[rocket::async_trait]
impl<O: Orbit> OrbitCollection<O> for Orbits<O> {
    async fn orbits(&self) -> RwLockReadGuard<BTreeMap<Cid, O>> {
        self.stores.read().await
    }

    // fn orbit(&self, id: &Cid) -> Option<&O> {
    //     self.stores.read().expect("read orbit set").get(id)
    // }

    async fn add(&self, orbit: O) {
        let mut lock = self.stores.write().await;
        lock.insert(*orbit.id(), orbit);
    }
}

async fn load_orbits<P: AsRef<Path>>(
    path: P,
) -> Result<Orbits<SimpleOrbit<TezosBasicAuthorization>>> {
    let path_ref: &Path = path.as_ref();
    let orbits = Orbits::new(path_ref);
    // for entries in the dir
    let orbit_list: Vec<SimpleOrbit<TezosBasicAuthorization>> =
        ReadDirStream::new(read_dir(path_ref).await?)
            // filter for those with valid CID filenames
            .filter_map(|p| async { Cid::from_str(p.ok()?.file_name().to_str()?).ok() })
            // get a future to load each
            .filter_map(|cid| async move {
                create_orbit(cid, path_ref, TezosBasicAuthorization)
                    .await
                    .ok()
            })
            // load them all
            .collect()
            .await;

    for orbit in orbit_list.into_iter() {
        orbits.add(orbit).await
    }

    Ok(orbits)
}

#[get("/<orbit_id>/<hash>")]
async fn get_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    _auth: Option<AuthWrapper<TZAuth>>,
) -> Result<Option<RocketStream<Cursor<Vec<u8>>>>, Debug<Error>> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or(anyhow!("No Orbit Found"))?;
    match ContentAddressedStorage::get(orbit, &hash.0).await {
        Ok(Some(content)) => Ok(Some(RocketStream::chunked(
            Cursor::new(content.to_owned()),
            1024,
        ))),
        Ok(None) => Ok(None),
        Err(e) => Err(e)?,
    }
}

#[post("/<orbit_id>", format = "multipart/form-data", data = "<batch>")]
async fn batch_put_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    batch: Form<Vec<PutContent>>,
    _auth: AuthWrapper<TZAuth>,
) -> Result<String, Debug<Error>> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or(anyhow!("No Orbit Found"))?;
    let mut cids = Vec::<String>::new();
    for content in batch.into_inner().into_iter() {
        cids.push(
            orbit
                .put(&content.content, content.codec)
                .await
                .map_or("".into(), |cid| {
                    cid.to_string_of_base(Base::Base58Btc)
                        .map_or("".into(), |s| s)
                }),
        );
    }
    Ok(cids.join("\n"))
}

#[post("/<orbit_id>", data = "<data>", rank = 2)]
async fn put_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    data: Data,
    codec: SupportedCodecs,
    _auth: AuthWrapper<TZAuth>,
) -> Result<String, Debug<Error>> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or(anyhow!("No Orbit Found"))?;
    match orbit
        .put(
            &data
                .open(10u8.megabytes())
                .into_bytes()
                .await
                .map_err(|e| anyhow!(e))?,
            codec,
        )
        .await
    {
        Ok(cid) => Ok(cid
            .to_string_of_base(Base::Base58Btc)
            .map_err(|e| anyhow!(e))?),
        Err(e) => Err(e)?,
    }
}

#[post("/", format = "multipart/form-data", data = "<batch>")]
async fn batch_put_create(
    // TODO find a good way to not restrict all orbits to the same Type
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    batch: Form<Vec<PutContent>>,
    auth: AuthWrapper<TZAuth>,
) -> Result<String, Debug<Error>> {
    match auth.0.action() {
        Action::Create {
            orbit_id,
            salt,
            content,
        } => {
            let orbit = create_orbit(*orbit_id, &orbits.base_path, TezosBasicAuthorization).await?;

            let mut cids = Vec::<String>::new();
            for mut content in batch.into_inner().into_iter() {
                cids.push(orbit.put(&mut content.content, content.codec).await.map_or(
                    "".into(),
                    |cid| {
                        cid.to_string_of_base(Base::Base58Btc)
                            .map_or("".into(), |s| s)
                    },
                ));
            }
            orbits.add(orbit);
            Ok(cids.join("\n"))
        }
        _ => Err(anyhow!("Invalid Authorization"))?,
    }
}

#[post("/", data = "<data>", rank = 2)]
async fn put_create(
    // TODO find a good way to not restrict all orbits to the same Type
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    data: Data,
    codec: SupportedCodecs,
    auth: AuthWrapper<TZAuth>,
) -> Result<String, Debug<Error>> {
    match auth.0.action() {
        Action::Create {
            orbit_id,
            salt,
            content,
        } => {
            let orbit = create_orbit(*orbit_id, &orbits.base_path, TezosBasicAuthorization).await?;

            let cid = orbit.put(&mut data.open(10u8.megabytes()), codec).await?;

            orbits.add(orbit);

            Ok(cid
                .to_string_of_base(Base::Base58Btc)
                .map_err(|e| anyhow!(e))?)
        }
        _ => Err(anyhow!("Invalid Authorization"))?,
    }
}

#[delete("/<orbit_id>/<hash>")]
async fn delete_content(
    orbits: State<'_, Orbits<SimpleOrbit<TezosBasicAuthorization>>>,
    orbit_id: CidWrap,
    hash: CidWrap,
    auth: AuthWrapper<TZAuth>,
) -> Result<(), Debug<Error>> {
    let orbits_read = orbits.orbits().await;
    let orbit = orbits_read
        .get(&orbit_id.0)
        .ok_or(anyhow!("No Orbit Found"))?;
    Ok(orbit.delete(&hash.0).await?)
}

#[rocket::main]
async fn main() -> Result<()> {
    let rocket_config = rocket::Config::figment();

    #[derive(Deserialize, Debug)]
    struct DBConfig {
        db_path: std::path::PathBuf,
    }

    let path = rocket_config
        .extract::<DBConfig>()
        .expect("db path missing")
        .db_path;

    rocket::custom(rocket_config)
        .manage(load_orbits(path).await?)
        .manage(TezosBasicAuthorization)
        .mount(
            "/",
            routes![
                get_content,
                put_content,
                batch_put_content,
                delete_content,
                put_create,
                batch_put_create
            ],
        )
        .attach(AdHoc::on_response("CORS", |_, resp| {
            Box::pin(async move {
                resp.set_header(Header::new("Access-Control-Allow-Origin", "*"));
                resp.set_header(Header::new(
                    "Access-Control-Allow-Methods",
                    "POST, GET, OPTIONS, DELETE",
                ));
                resp.set_header(Header::new("Access-Control-Allow-Headers", "*"));
                resp.set_header(Header::new("Access-Control-Allow-Credentials", "true"));
            })
        }))
        .launch()
        .await?;

    Ok(())
}

#[test]
async fn test_form() {
    use rocket::{http::ContentType, local::blocking::Client};
    let cid1 = "mAYAEFiAGoa04sUYQ7G8LD2+Rx1Tc2aOCVzj6Sw+tQJ8j20S52Q";
    let cid2 = "mAYAEFiD5JYqnqRB2lKxvMzl17mpZlcdUR7aRjhP1zyECXgr0XA";
    let p1: (Cid, String) = (
        cid1.parse().unwrap(),
        r#"{"@context":["https://www.w3.org/2018/credentials/v1",{"BasicProfile":"https://tzprofiles.me/BasicProfile","logo":"https://schema.org/logo","website":"https://schema.org/url","description":"https://schema.org/description","alias":"https://schema.org/name"}],"id":"urn:uuid:a8d2fa78-49f3-44e7-b8dc-2592917455c1","type":["VerifiableCredential","BasicProfile"],"credentialSubject":{"id":"did:pkh:tz:tz1YSb7gXhgBw46nSXthhoSzhJdbQf9h92Gy","alias":"gfdwgd","logo":"fdsgdsg","description":"fdsgfads","website":"fdsagds"},"issuer":"did:pkh:tz:tz1YSb7gXhgBw46nSXthhoSzhJdbQf9h92Gy","issuanceDate":"2021-04-27T10:30:53.939Z","proof":{"@context":{"TezosMethod2021":"https://w3id.org/security#TezosMethod2021","TezosSignature2021":{"@context":{"@protected":true,"@version":1.1,"challenge":"https://w3id.org/security#challenge","created":{"@id":"http://purl.org/dc/terms/created","@type":"http://www.w3.org/2001/XMLSchema#dateTime"},"domain":"https://w3id.org/security#domain","expires":{"@id":"https://w3id.org/security#expiration","@type":"http://www.w3.org/2001/XMLSchema#dateTime"},"id":"@id","nonce":"https://w3id.org/security#nonce","proofPurpose":{"@context":{"@protected":true,"@version":1.1,"assertionMethod":{"@container":"@set","@id":"https://w3id.org/security#assertionMethod","@type":"@id"},"authentication":{"@container":"@set","@id":"https://w3id.org/security#authenticationMethod","@type":"@id"},"id":"@id","type":"@type"},"@id":"https://w3id.org/security#proofPurpose","@type":"@vocab"},"proofValue":"https://w3id.org/security#proofValue","publicKeyJwk":{"@id":"https://w3id.org/security#publicKeyJwk","@type":"@json"},"type":"@type","verificationMethod":{"@id":"https://w3id.org/security#verificationMethod","@type":"@id"}},"@id":"https://w3id.org/security#TezosSignature2021"}},"type":"TezosSignature2021","proofPurpose":"assertionMethod","proofValue":"edsigtdmHmWuXsdCqnkWnc5QJuUAk9tLTd73JjRJEL7qrC79iSStj91AU3U1faq85XhqgHyNoLBh6Fqod415aovQh73dRcbbpFj","verificationMethod":"did:pkh:tz:tz1YSb7gXhgBw46nSXthhoSzhJdbQf9h92Gy#TezosMethod2021","created":"2021-04-27T10:30:53.940Z","publicKeyJwk":{"alg":"EdDSA","crv":"Ed25519","kty":"OKP","x":"tA2T93-4HFNQ7TIfWyN-nXOgqbO5M9NJLB_JsTRXuwI"}}}"#.into()
    );
    let p2: (Cid, String) = (cid2.parse().unwrap(), r#"{"dummy":"obj"}"#.into());

    #[post("/", data = "<form>")]
    async fn stub_batch(form: Form<BTreeMap<CidWrap, PutContent>>) {
        let cid1 = "mAYAEFiAGoa04sUYQ7G8LD2+Rx1Tc2aOCVzj6Sw+tQJ8j20S52Q";
        let cid2 = "mAYAEFiD5JYqnqRB2lKxvMzl17mpZlcdUR7aRjhP1zyECXgr0XA";
        let p1: (Cid, String) = (
            cid1.parse().unwrap(),
            r#"{"@context":["https://www.w3.org/2018/credentials/v1",{"BasicProfile":"https://tzprofiles.me/BasicProfile","logo":"https://schema.org/logo","website":"https://schema.org/url","description":"https://schema.org/description","alias":"https://schema.org/name"}],"id":"urn:uuid:a8d2fa78-49f3-44e7-b8dc-2592917455c1","type":["VerifiableCredential","BasicProfile"],"credentialSubject":{"id":"did:pkh:tz:tz1YSb7gXhgBw46nSXthhoSzhJdbQf9h92Gy","alias":"gfdwgd","logo":"fdsgdsg","description":"fdsgfads","website":"fdsagds"},"issuer":"did:pkh:tz:tz1YSb7gXhgBw46nSXthhoSzhJdbQf9h92Gy","issuanceDate":"2021-04-27T10:30:53.939Z","proof":{"@context":{"TezosMethod2021":"https://w3id.org/security#TezosMethod2021","TezosSignature2021":{"@context":{"@protected":true,"@version":1.1,"challenge":"https://w3id.org/security#challenge","created":{"@id":"http://purl.org/dc/terms/created","@type":"http://www.w3.org/2001/XMLSchema#dateTime"},"domain":"https://w3id.org/security#domain","expires":{"@id":"https://w3id.org/security#expiration","@type":"http://www.w3.org/2001/XMLSchema#dateTime"},"id":"@id","nonce":"https://w3id.org/security#nonce","proofPurpose":{"@context":{"@protected":true,"@version":1.1,"assertionMethod":{"@container":"@set","@id":"https://w3id.org/security#assertionMethod","@type":"@id"},"authentication":{"@container":"@set","@id":"https://w3id.org/security#authenticationMethod","@type":"@id"},"id":"@id","type":"@type"},"@id":"https://w3id.org/security#proofPurpose","@type":"@vocab"},"proofValue":"https://w3id.org/security#proofValue","publicKeyJwk":{"@id":"https://w3id.org/security#publicKeyJwk","@type":"@json"},"type":"@type","verificationMethod":{"@id":"https://w3id.org/security#verificationMethod","@type":"@id"}},"@id":"https://w3id.org/security#TezosSignature2021"}},"type":"TezosSignature2021","proofPurpose":"assertionMethod","proofValue":"edsigtdmHmWuXsdCqnkWnc5QJuUAk9tLTd73JjRJEL7qrC79iSStj91AU3U1faq85XhqgHyNoLBh6Fqod415aovQh73dRcbbpFj","verificationMethod":"did:pkh:tz:tz1YSb7gXhgBw46nSXthhoSzhJdbQf9h92Gy#TezosMethod2021","created":"2021-04-27T10:30:53.940Z","publicKeyJwk":{"alg":"EdDSA","crv":"Ed25519","kty":"OKP","x":"tA2T93-4HFNQ7TIfWyN-nXOgqbO5M9NJLB_JsTRXuwI"}}}"#.into()
        );
        let p2: (Cid, String) = (cid2.parse().unwrap(), r#"{"dummy":"obj"}"#.into());
        let content1 = &form.get(&CidWrap(p1.0)).unwrap().content;
        let content2 = &form.get(&CidWrap(p2.0)).unwrap().content;
        assert_eq!(&content1.value, p1.1.as_bytes());
        assert_eq!(&content2.value, p2.1.as_bytes());
    }
    let boundary = "-----------------------------61504105631770370051895920508";
    let pre = r#"Content-Disposition: form-data; name=""#;
    let post = r#""; filename="blob""#;
    let ct = "Content-Type: application/json";

    let form = [
        boundary,
        &format!("{}{}{}", pre, cid1, post),
        ct,
        "",
        &p1.1,
        boundary,
        &format!("{}{}{}", pre, cid2, post),
        ct,
        "",
        &p2.1,
        &format!("{}--", boundary),
    ]
    .join("\n\r");

    let client = Client::debug_with(rocket::routes![stub_batch]).unwrap();
    let res = client
        .post("/")
        .header(
            format!("multipart/form-data; boundary={}", boundary)
                .parse::<ContentType>()
                .unwrap(),
        )
        .body(&form)
        .dispatch();

    assert!(res.status().class().is_success());
}
