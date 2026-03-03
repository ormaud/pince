FROM ghcr.io/trame-sh/trame-tools:latest

# Rust toolchain
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
ENV PATH="/usr/local/cargo/bin:$PATH"
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
