FROM clux/muslrust:stable AS chef
USER root
RUN cargo install cargo-chef
WORKDIR /app

FROM chef AS planner
COPY ./Cargo.lock ./
COPY ./Cargo.toml ./
COPY ./sdk-wasm/ ./sdk-wasm/
COPY ./src/ ./src/
COPY ./lib/ ./lib/
COPY ./sdk/ ./sdk/
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --target x86_64-unknown-linux-musl --recipe-path recipe.json
COPY --from=planner /app/ ./
RUN cargo build --release --target x86_64-unknown-linux-musl --bin kepler

FROM alpine AS runtime
RUN addgroup -S kepler && adduser -S kepler -G kepler
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/kepler /usr/local/bin/kepler
USER kepler
COPY ./kepler.toml ./
ENV ROCKET_ADDRESS=0.0.0.0
EXPOSE 8000
EXPOSE 8001
EXPOSE 8081
CMD ["kepler"]
LABEL org.opencontainers.image.source https://github.com/spruceid/kepler
