FROM vitasdk/vitasdk@sha256:acd15285e1ae473e925c6f83fa41e2de45eb8b9e8ba89be7df03e194c35c11d3
ARG RUST_TOOLCHAIN=nightly-2026-01-20
ENV PATH=/root/.cargo/bin:/usr/local/vitasdk/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh \
    && sh /tmp/rustup.sh -y --profile minimal --default-toolchain ${RUST_TOOLCHAIN} --component rust-src
WORKDIR /src
