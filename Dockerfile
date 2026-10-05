# Dockerfile для Idenity API (Rust/Axum)
# Используем multi-stage build для минимального размера образа

# Stage 1: Builder
FROM rust:1.75-slim-bookworm AS builder

WORKDIR /app

# Устанавливаем системные зависимости
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Копируем Cargo файлы отдельно для кэширования слоёв
COPY Cargo.toml Cargo.lock ./
COPY api/Cargo.toml ./api/
COPY api/src ./api/src

# Собираем релизный бинарник
RUN cargo build --release -p idenity-api

# Stage 2: Runtime
FROM debian:bookworm-slim

WORKDIR /app

# Устанавливаем только runtime-зависимости
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Копируем бинарник из builder
COPY --from=builder /app/target/release/idenity-api /app/idenity-api

# Render назначает PORT автоматически через env var
# Бэкенд читает PORT из env (дефолт 8080)
EXPOSE 8080

# Запускаем бэкенд
CMD ["/app/idenity-api"]
