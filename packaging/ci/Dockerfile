# CI image with every toolchain the binding tests need, pre-installed — so the
# Gitea jobs skip ~all of their setup steps (and the flaky downloads that go with
# them: rustup, setup-go/-dotnet/-java/-dart/-node, haskell, cargo-install of nu
# and of every binding generator).
#
#   just ci-image                       # build
#   just ci-image-push                  # push to the Gitea container registry
#
# Use it in a workflow with:
#   jobs:
#     test-bindings-runtime:
#       container: 192.168.1.44:3000/sorin/stem-mqtt-ci:latest
# and drop the setup-*/rustup/nushell/install_bindgens steps.
#
# NOTE: written for linux/amd64; not built in the repo's own CI (the runners here
# have no Docker), so re-check version pins when you first build it.
FROM ubuntu:24.04

ARG NU_VERSION=0.111.0
ARG GO_VERSION=1.24.6
ARG JDK_MAJOR=22
ARG NODE_MAJOR=22
ARG GRADLE_VERSION=8.14.3
ARG GHC_VERSION=9.10.2
ARG CABAL_VERSION=3.12.1.0

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo \
    GHCUP_INSTALL_BASE_PREFIX=/opt \
    BINDGEN_ROOT=/opt/bindgens \
    DOTNET_ROOT=/opt/dotnet DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 \
    JAVA_HOME=/opt/jdk \
    PATH=/opt/cargo/bin:/opt/bindgens/bin:/opt/go/bin:/opt/dotnet:/opt/jdk/bin:/opt/dart-sdk/bin:/opt/gradle/bin:/opt/.ghcup/bin:/opt/node/bin:$PATH

RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git unzip xz-utils build-essential pkg-config cmake \
      python3 python3-venv libffi-dev libgmp-dev libncurses-dev zlib1g-dev \
      gcc-aarch64-linux-gnu libc6-dev-arm64-cross sudo \
    && rm -rf /var/lib/apt/lists/*

# Rust (minimal profile: the binding jobs only compile)
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable

# Nushell (prebuilt, instead of a ~10 minute `cargo install nu`)
RUN curl -fsSL "https://github.com/nushell/nushell/releases/download/${NU_VERSION}/nu-${NU_VERSION}-x86_64-unknown-linux-gnu.tar.gz" \
    | tar xz -C /tmp && install -m755 /tmp/nu-*/nu /usr/local/bin/nu && rm -rf /tmp/nu-*

# Go, JDK (Temurin; the Java bindings need >= 22), Node, Dart, Gradle, .NET 8
RUN curl -fsSL "https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz" | tar xz -C /opt \
 && mkdir -p /opt/jdk && curl -fsSL "https://api.adoptium.net/v3/binary/latest/${JDK_MAJOR}/ga/linux/x64/jdk/hotspot/normal/eclipse" | tar xz -C /opt/jdk --strip-components=1 \
 && mkdir -p /opt/node && curl -fsSL "https://nodejs.org/dist/latest-v${NODE_MAJOR}.x/$(curl -fsSL https://nodejs.org/dist/latest-v${NODE_MAJOR}.x/SHASUMS256.txt | grep -o 'node-v[0-9.]*-linux-x64.tar.xz' | head -1)" | tar xJ -C /opt/node --strip-components=1 \
 && curl -fsSL https://storage.googleapis.com/dart-archive/channels/stable/release/latest/sdk/dartsdk-linux-x64-release.zip -o /tmp/dart.zip && unzip -q /tmp/dart.zip -d /opt && rm /tmp/dart.zip \
 && curl -fsSL "https://services.gradle.org/distributions/gradle-${GRADLE_VERSION}-bin.zip" -o /tmp/gradle.zip && unzip -q /tmp/gradle.zip -d /opt && mv /opt/gradle-${GRADLE_VERSION} /opt/gradle && rm /tmp/gradle.zip \
 && curl -fsSL https://dot.net/v1/dotnet-install.sh -o /tmp/dotnet-install.sh && bash /tmp/dotnet-install.sh --channel 8.0 --install-dir /opt/dotnet && rm /tmp/dotnet-install.sh

# Haskell (ghcup)
RUN curl --proto '=https' --tlsv1.2 -sSf https://get-ghcup.haskell.org | BOOTSTRAP_HASKELL_NONINTERACTIVE=1 BOOTSTRAP_HASKELL_GHC_VERSION=${GHC_VERSION} BOOTSTRAP_HASKELL_CABAL_VERSION=${CABAL_VERSION} BOOTSTRAP_HASKELL_ADJUST_BASHRC=0 sh \
 && cabal update

# The pinned third-party binding generators (versions live in scripts/bindings/spec.nu).
# Only the scripts are needed to install them — no workspace sources.
COPY scripts /opt/stem-mqtt/scripts
COPY packaging/haskell /opt/stem-mqtt/packaging/haskell
RUN cd /opt/stem-mqtt && nu scripts/install_bindgens.nu --root "$BINDGEN_ROOT" \
 && rm -rf /opt/stem-mqtt/target /opt/cargo/registry /opt/cargo/git

WORKDIR /workspace
