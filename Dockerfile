FROM alpine:3 AS build

RUN apk update && \
    apk add --no-cache \
    build-base \
    curl

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

WORKDIR /usr/app

COPY src/* ./src/
COPY Cargo.* ./

RUN source $HOME/.cargo/env && cargo build --release

FROM alpine:3

WORKDIR /usr/app

COPY --from=build /usr/app/target/release/rinha-backend-rust-pg /usr/bin/

ENTRYPOINT [ "rinha-backend-rust-pg" ]