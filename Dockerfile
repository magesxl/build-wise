FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release

FROM alpine:3.21
RUN apk add --no-cache ca-certificates nodejs npm
WORKDIR /app
COPY --from=builder /app/target/release/build-wise .
COPY config.yaml .
EXPOSE 3000
CMD ["./build-wise"]
