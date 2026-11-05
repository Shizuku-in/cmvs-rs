FROM devkitpro/devkita64@sha256:1fc388c3a0d34bd2045a6dadcb1020e069d5f876a187fd705de14b4440c00282
ARG RUST_TOOLCHAIN=nightly-2026-01-20
ENV PATH=/root/.cargo/bin:/opt/devkitpro/devkitA64/bin:/opt/devkitpro/tools/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh \
    && sh /tmp/rustup.sh -y --profile minimal --default-toolchain ${RUST_TOOLCHAIN} --component rust-src
WORKDIR /src
