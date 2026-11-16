FROM oven/bun:1.4.0 AS bun
FROM rust:1.100.0-bookworm
COPY --from=bun /usr/local/bin/bun /usr/local/bin/bun
RUN apt-get update && apt-get install -y --no-install-recommends locales git musl-tools \
    && sed -i '/en_US.UTF-8/s/^# //g' /etc/locale.gen && locale-gen \
    && rustup component add clippy rustfmt \
    && case "$(uname -m)" in aarch64) rustup target add aarch64-unknown-linux-musl ;; x86_64) rustup target add x86_64-unknown-linux-musl ;; esac
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc
ENV LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 BUN_RUNTIME_TRANSPILER_CACHE_PATH=""
# Run with a nonroot --user; EACCES scenarios must exercise permissions.
