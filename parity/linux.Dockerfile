FROM oven/bun:1.4.0 AS bun
FROM rust:1.100.0-bookworm
COPY --from=bun /usr/local/bin/bun /usr/local/bin/bun
RUN apt-get update && apt-get install -y --no-install-recommends locales git \
    && sed -i '/en_US.UTF-8/s/^# //g' /etc/locale.gen && locale-gen \
    && rustup component add clippy rustfmt
ENV LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 BUN_RUNTIME_TRANSPILER_CACHE_PATH=""
# Run with a nonroot --user; EACCES scenarios must exercise permissions.
