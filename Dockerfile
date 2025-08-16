# syntax=docker/dockerfile:1

FROM rust:1.89-slim-trixie AS chef
RUN cargo install cargo-chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Build environment
FROM chef AS build

SHELL ["/bin/bash", "-eu", "-o", "pipefail", "-c"]
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked --mount=type=cache,target=/var/lib/apt,sharing=locked <<EOF
    apt-get update
    apt-get install -y \
        build-essential \
        checkinstall \
        zlib1g-dev \
        pkg-config \
        libzstd-dev \
        libssl-dev \
        --no-install-recommends
EOF

RUN <<EOF
    mkdir -p /app
    useradd appuser
    chown -R appuser: /app
    mkdir -p /home/appuser
    chown -R appuser: /home/appuser
EOF

COPY --from=planner /app/recipe.json recipe.json

# Build dependencies - this is the caching Docker layer
RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --frozen

# Production
FROM docker.io/debian:trixie-slim AS prod

SHELL ["/bin/bash", "-eu", "-o", "pipefail", "-c"]
RUN <<EOF
    mkdir -p /app
    useradd appuser
    chown -R appuser: /app
    mkdir -p /home/appuser
    chown -R appuser: /home/appuser
EOF

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked --mount=type=cache,target=/var/lib/apt,sharing=locked <<EOF
    apt-get update
    apt-get upgrade -y
    apt-get install -y \
        ca-certificates \
        libssl3t64=3.* \
        dumb-init \
        --no-install-recommends
    update-ca-certificates
EOF

USER appuser

COPY --from=build /app/target/release/envoyproxy /usr/local/bin/envoyproxy

ENTRYPOINT ["/usr/bin/dumb-init"]
ARG ENVOY_JWT
LABEL org.opencontainers.image.source=https://github.com/Roguelazer/envoyproxy
CMD /usr/local/bin/envoyproxy
