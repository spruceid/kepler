use kepler::{app, config};
use rocket::figment::providers::{Format, Serialized, Toml, Env};

#[rocket::main]
async fn main() {
    let config = rocket::figment::Figment::from(rocket::Config::default())
        .merge(Serialized::defaults(config::Config::default()))
        .merge(Toml::file("kepler.toml").nested())
        .merge(Env::prefixed("KEPLER_").split("_").global())
        .merge(Env::prefixed("ROCKET_").global()); // That's just for easy access to ROCKET_LOG_LEVEL

    app(&config).await.unwrap().launch().await.unwrap();
}
