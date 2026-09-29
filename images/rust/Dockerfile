# Rust project layer.
ARG BASE_IMAGE=moor/base:latest
FROM ${BASE_IMAGE}

USER root
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
USER agent

ENV RUSTUP_HOME=/home/agent/.rustup \
    CARGO_HOME=/home/agent/.cargo \
    PATH=/home/agent/.cargo/bin:$PATH

# See images/base/Dockerfile for why pipefail matters for a curl | sh RUN.
SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# clippy and rustfmt on top of `minimal`: a project's checks run
# `cargo clippy`, and without the component it can never pass.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --component clippy,rustfmt --default-toolchain stable

# At runtime the root filesystem is read-only, so cargo's registry and git
# checkouts go to the project's cache volume (/home/agent/.cache) instead
# of CARGO_HOME's install-time location. Without this, any build that
# needs a crate not already downloaded fails before compiling anything.
# The toolchain's binaries stay in /home/agent/.cargo/bin, on PATH.
ENV CARGO_HOME=/home/agent/.cache/cargo
