# Start with Ubuntu base image
FROM ubuntu:22.04

# Prevent timezone prompt during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Install system dependencies
RUN apt-get update && apt-get install -y \
    curl \
    build-essential \
    pkg-config \
    cmake \
    libopencv-dev \
    python3-opencv \
    clang \
    libclang-dev \
    ffmpeg \
    && rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Create working directory
WORKDIR /app

# Copy the source code first
COPY ./src ./src
COPY ./benches ./benches
COPY ./tests ./tests
COPY ./Cargo.toml ./Cargo.lock ./

# Create assets directory (but don't copy assets)
RUN mkdir -p /app/assets

# Build the project
RUN cargo build --release

# Create Unix socket directory
RUN mkdir -p /tmp

# Expose WebSocket port
EXPOSE 5500

# Run the application
CMD ["./target/release/vid-tool"]
