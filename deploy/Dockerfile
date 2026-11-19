# One image: the API server and the built web app.
# Not built in CI yet. Build with: docker build -f deploy/Dockerfile -t scaffoldry .

# Stage 1: build the web app.
FROM node:22-alpine AS web
WORKDIR /app
COPY package*.json ./
COPY apps/web/package*.json ./apps/web/
RUN npm ci --workspace=@scaffoldry/web
COPY apps/web ./apps/web
WORKDIR /app/apps/web
RUN npx vite build

# Stage 2: build the server.
FROM rust:1-bookworm AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p scaffoldry-server

# Stage 3: small runtime image, non-root.
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home scaffoldry
COPY --from=server /src/target/release/scaffoldry-server /usr/local/bin/scaffoldry-server
COPY --from=web /app/apps/web/dist /srv/web
# Used only by the playtest service (deploy/cloudrun-playtest.yaml). The default command is still the server.
COPY deploy/playtest-entrypoint.sh /usr/local/bin/playtest-entrypoint.sh
ENV SCAFFOLDRY_WEB_DIR=/srv/web HOST=0.0.0.0 PORT=8080
USER scaffoldry
EXPOSE 8080
CMD ["scaffoldry-server"]
