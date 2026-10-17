# syntax=docker/dockerfile:1

# Builder and runtime must share the Debian release (glibc version).
FROM lukemathwalker/cargo-chef:latest-rust-1-trixie AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Dependencies are cached in this layer as long as Cargo.lock is unchanged.
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --bin test-app

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=builder /app/target/release/test-app /test-app
ENV TESTAPP_BIND=0.0.0.0:8080 \
    TESTAPP_LOG_FORMAT=json
EXPOSE 8080
USER nonroot
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD ["/test-app", "healthcheck"]
ENTRYPOINT ["/test-app"]
CMD ["serve"]
