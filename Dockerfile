ARG DIR=/kepler
ARG BASE_IMAGE=ekidd/rust-musl-builder

# Dependencies cache
FROM $BASE_IMAGE as dep_planner
ARG DIR
WORKDIR $DIR
USER root
RUN chown -R rust:rust .
USER rust
RUN cargo install cargo-chef
COPY ./Cargo.lock ./
COPY ./Cargo.toml ./
COPY ./src/ ./src/
COPY ./lib/ ./lib/
COPY ./sdk-wasm/ ./sdk-wasm/
RUN cargo chef prepare  --recipe-path recipe.json

FROM $BASE_IMAGE as dep_cacher
ARG DIR
WORKDIR $DIR
USER root
RUN chown -R rust:rust .
USER rust
RUN cargo install cargo-chef
COPY --from=dep_planner ${DIR}/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

FROM $BASE_IMAGE as builder
ARG DIR
WORKDIR $DIR
USER root
RUN chown -R rust:rust .
USER rust
COPY --from=dep_planner ${DIR}/ ./
COPY --from=dep_cacher ${DIR}/target/ ./target/
COPY --from=dep_cacher $CARGO_HOME $CARGO_HOME
RUN cargo build --release

FROM alpine
ARG DIR
WORKDIR $DIR
COPY --from=builder ${DIR}/target/x86_64-unknown-linux-musl/release/kepler /usr/local/bin/
COPY ./kepler.toml ./
ENV ROCKET_ADDRESS=0.0.0.0
EXPOSE 8000
EXPOSE 8001
EXPOSE 8081
ENTRYPOINT ["kepler"]
LABEL org.opencontainers.image.source https://github.com/spruceid/kepler
