# syntax=docker/dockerfile:1
# ----- builder -----
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev pkgconfig openssl-dev

COPY --from=oven/bun:alpine /usr/local/bin/bun /usr/local/bin/bun

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY frontend ./frontend
# Needed at COMPILE time: src/sp500.rs embeds universe/sp500.txt via include_str!
# (the runtime stage also copies universe/ for starter.csv, read at runtime).
COPY universe ./universe

RUN cd frontend && bun install --frozen-lockfile && bun run build

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && \
    cp target/release/finance /app/finance

# ----- runtime -----
FROM alpine:3.23

# Outbound HTTPS to Yahoo and SEC EDGAR.
RUN apk add --no-cache ca-certificates

WORKDIR /app

COPY --from=builder /app/finance ./finance
COPY --from=builder /app/dist ./dist
COPY templates ./templates
COPY migrations ./migrations
COPY universe ./universe

RUN addgroup -S -g 1000 app && \
    adduser -S -h /app -s /sbin/nologin -u 1000 -G app app && \
    mkdir -p /data && chown -R app:app /app /data
USER app

ENV PORT=8000
ENV FINANCE_DATA_DIR=/data
EXPOSE 8000

CMD ["./finance"]
