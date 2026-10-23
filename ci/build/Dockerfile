# Reproducible release build image (issue #1614).
#
# Pins rustc (via the base image tag, matching rust-toolchain.toml), the
# wasm32v1-none target and stellar-cli. `stellar contract optimize` bundles its
# own wasm-opt, so pinning stellar-cli pins the optimizer too.
#
# Build:  docker build -t scoutchain-wasm-build -f ci/build/Dockerfile ci/build
# Run:    docker run --rm -v "$PWD:/src" -v "$PWD/out:/out" scoutchain-wasm-build /out
FROM rust:1.97.1-slim-bookworm

ARG STELLAR_CLI_VERSION=28.0.0

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates git pkg-config libssl-dev libdbus-1-dev libudev-dev \
    && rm -rf /var/lib/apt/lists/*

RUN rustup target add wasm32v1-none \
    && cargo install --locked stellar-cli --version "${STELLAR_CLI_VERSION}"

WORKDIR /src
ENV SOURCE_DATE_EPOCH=0
ENTRYPOINT ["/src/ci/build/build-wasm.sh"]
