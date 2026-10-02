FROM docker.io/library/rust:1-slim-trixie AS build
WORKDIR /src
# Cache dependencies separately from sources.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && touch src/lib.rs \
    && cargo build --release && rm -rf src
COPY migrations migrations
COPY src src
RUN touch src/lib.rs src/main.rs && cargo build --release

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=build /src/target/release/history /history
ENV HISTORY_LISTEN=0.0.0.0:8081
EXPOSE 8081
ENTRYPOINT ["/history"]
