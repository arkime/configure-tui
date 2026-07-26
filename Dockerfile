# Build + run Configure-tui in a container, so people can generate Arkime
# docker-compose.yml / arkime.env files without installing anything.
#
#   docker build -t configure-tui .
#   docker run --rm -it -v "$PWD:/work" configure-tui
#
# It writes the generated files into /work — mount the directory you want them
# in. Pick "Docker — create a new docker-compose" on the first screen.

FROM rust:alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN cargo build --release --locked

FROM alpine:3.20
LABEL org.opencontainers.image.source="https://github.com/arkime/configure-tui"
LABEL org.opencontainers.image.description="Interactive TUI to generate Arkime docker-compose / config files"
COPY --from=build /src/target/release/Configure-tui /usr/local/bin/Configure-tui
WORKDIR /work
ENTRYPOINT ["Configure-tui"]
