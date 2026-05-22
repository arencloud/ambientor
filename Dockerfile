# syntax=docker/dockerfile:1
FROM rust:1.95-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p ambientor-operator -p ambientor-api -p ambientor-web -p ambientor-cli

FROM gcr.io/distroless/cc-debian12:nonroot AS operator
COPY --from=builder /app/target/release/ambientor-operator /ambientor-operator
ENTRYPOINT ["/ambientor-operator"]

FROM gcr.io/distroless/cc-debian12:nonroot AS api
COPY --from=builder /app/target/release/ambientor-api /ambientor-api
ENTRYPOINT ["/ambientor-api"]

FROM gcr.io/distroless/cc-debian12:nonroot AS web
COPY --from=builder /app/target/release/ambientor-web /ambientor-web
ENTRYPOINT ["/ambientor-web"]

FROM gcr.io/distroless/cc-debian12:nonroot AS cli
COPY --from=builder /app/target/release/ambientor /ambientor
ENTRYPOINT ["/ambientor"]
