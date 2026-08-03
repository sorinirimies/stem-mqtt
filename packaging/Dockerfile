# syntax=docker/dockerfile:1
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY target/release/mqtt-broker /usr/local/bin/mqtt-broker
EXPOSE 1883
ENTRYPOINT ["mqtt-broker"]
CMD ["--bind", "0.0.0.0", "--port", "1883"]
