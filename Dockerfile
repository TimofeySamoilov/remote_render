FROM rust:1.79-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    libssl-dev \
    pkg-config \
    ca-certificates \
    libasound2-dev \
    libudev-dev \
    libx11-dev \
    libxi-dev \
    libxtst-dev \
    xvfb \
    mesa-utils \
    libgl1-mesa-dev \
    libxcursor-dev \
    libxrandr-dev \
    libxinerama-dev \
    libxxf86vm-dev \
    && rm -rf /var/lib/apt/lists/*

# Создаем рабочую директорию внутри контейнера. Это место, где будет ваш код.
WORKDIR /app

# Копируем proto файлы
COPY proto ./proto

# Копируем все исходники.
COPY ./src ./src

# Проверяем, что файлы скопированы
RUN ls -l /app/src
RUN cat /app/src/server_main.rs

# Копируем Cargo.toml, Cargo.lock и build.rs
COPY Cargo.toml Cargo.lock build.rs ./

# Собираем зависимости и генерируем protobuf код.  --release важен для оптимизации.
RUN cargo build --release --target x86_64-unknown-linux-gnu

# Собираем ТОЛЬКО бинарник сервера.
RUN cargo build --release --target x86_64-unknown-linux-gnu --bin remote_render-server_main

# Второй этап: создаем минимальный образ для запуска.
FROM debian:bookworm-slim

# Копируем скомпилированный бинарник из этапа сборки.
COPY --from=builder /app/target/x86_64-unknown-linux-gnu/release/remote_render-server_main /usr/local/bin/remote_render-server_main

# Устанавливаем openssl, так как без него gRPC клиент не заработает!
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 \
    ca-certificates \
    libasound2 \
    libudev1 \
    libx11-6 \
    libxi6 \
    libxtst6 \
    xvfb \
    mesa-utils \
    libgl1-mesa-dev \
    libxcursor-dev \
    libxrandr-dev \
    libxinerama-dev \
    libxxf86vm-dev \
    && rm -rf /var/lib/apt/lists/*

# Объявляем порт, который будет слушать сервер
EXPOSE 50051

# Запускаем сервер.
CMD ["/usr/local/bin/remote_render-server_main"]
