FROM rust:1.70 as builder

# Install OpenCV dependencies
RUN apt-get update && apt-get install -y \
    libopencv-dev \
    clang \
    cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifest files
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src

# Build the application
RUN cargo build --release

# Runtime stage
FROM debian:bullseye-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libopencv-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the built binary from builder
COPY --from=builder /app/target/release/vid-tool /app/vid-tool

# Create directory for assets
RUN mkdir -p /app/assets

# Run the video processor
CMD ["./vid-tool"]
