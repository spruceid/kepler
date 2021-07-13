FROM rust:latest
MAINTAINER Charles Cunningham <charles.cunningham@spruceid.com>

WORKDIR /kepler

COPY . .

RUN cargo build --release

ENTRYPOINT ["./target/release/kepler"]
