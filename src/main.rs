use hyper::{
    service::{make_service_fn, service_fn},
    Server,
};
use kepler::{app, config, prometheus, tracing_try_init};
use rocket::{
    figment::providers::{Env, Format, Serialized, Toml},
    tokio,
};

#[rocket::main]
async fn main() {
    tracing_try_init();

    let config = rocket::figment::Figment::from(rocket::Config::default())
        .merge(Serialized::defaults(config::Config::default()))
        .merge(Toml::file("kepler.toml").nested())
        .merge(Env::prefixed("KEPLER_").split("_").global())
        .merge(Env::prefixed("ROCKET_").global()); // That's just for easy access to ROCKET_LOG_LEVEL
    let kepler_config = config.extract::<config::Config>().unwrap();

    let rocket = app(&config).await.unwrap().ignite().await.unwrap();

    let prom_addr = (rocket.config().address, kepler_config.prometheus.port).into();
    let prometheus = Server::bind(&prom_addr).serve(make_service_fn(|_| async {
        Ok::<_, hyper::Error>(service_fn(prometheus::serve_req))
    }));

    tokio::select! {
        r = rocket.launch() => r.unwrap(),
        r = prometheus => r.unwrap()
    };
}
