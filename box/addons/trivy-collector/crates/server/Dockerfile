# Statically linked binary built via GitHub Actions cross-compilation (cargo-zigbuild)
FROM alpine:3.23 AS certs
RUN apk add --no-cache ca-certificates

FROM scratch AS runtime

LABEL org.opencontainers.image.title="trivy-collector-server" \
      org.opencontainers.image.description="trivy-collector server: web UI, REST API, and MCP server over the scraper's reports" \
      org.opencontainers.image.version="1.8.3" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.base.name="scratch" \
      org.opencontainers.image.deprecated="false"

ARG TARGETARCH
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --chmod=755 trivy-collector-server-linux-${TARGETARCH} /app/trivy-collector-server
# 3000 UI and API, 8080 health and metrics.
EXPOSE 3000 8080
ENV SERVER_PORT=3000 \
    HEALTH_PORT=8080 \
    LOG_FORMAT=json
USER 1000:1000
ENTRYPOINT ["/app/trivy-collector-server"]
