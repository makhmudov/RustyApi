# syntax=docker/dockerfile:1
#
# Многоэтапная сборка: в итоговом образе только бинарник, миграции и корневые сертификаты.
# Собирается из корня репозитория (пример зависит от rustyapi по пути):
#   docker build -f examples/production/Dockerfile -t shortener .
# В своём проекте (`tez new`) контекст — корень проекта, а пути `examples/production/` убираются.

FROM rust:1-slim-bookworm AS builder
WORKDIR /src
COPY . .
# Кэш реестра crates и target между сборками (BuildKit): пересборка после правки кода — секунды, а не минуты.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p shortener \
    && cp target/release/shortener /usr/local/bin/shortener

FROM debian:bookworm-slim
# Сертификаты — из образа сборки: без apt-get итоговый этап собирается без сети и быстрее.
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
WORKDIR /app
COPY --from=builder /usr/local/bin/shortener /usr/local/bin/shortener
COPY examples/production/migrations ./migrations
ENV HOST=0.0.0.0 \
    PORT=8000 \
    MIGRATIONS_DIR=/app/migrations \
    RUST_LOG=info
USER 10001:10001
EXPOSE 8000
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD ["shortener", "healthcheck"]
CMD ["shortener"]
