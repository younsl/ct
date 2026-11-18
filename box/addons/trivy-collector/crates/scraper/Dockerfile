# Statically linked binary built via GitHub Actions cross-compilation (cargo-zigbuild)
FROM alpine:3.23 AS certs
RUN apk add --no-cache ca-certificates

FROM scratch AS runtime

LABEL org.opencontainers.image.title="trivy-collector-scraper" \
      org.opencontainers.image.description="trivy-collector scraper: watches Trivy Operator reports across clusters and owns the report database" \
      org.opencontainers.image.version="1.8.3" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.base.name="scratch" \
      org.opencontainers.image.deprecated="false"

ARG TARGETARCH
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --chmod=755 trivy-collector-scraper-linux-${TARGETARCH} /app/trivy-collector-scraper
# 8080 health and metrics, 8081 internal read API (never exposed outside the namespace).
EXPOSE 8080 8081
ENV STORAGE_PATH=/data \
    HEALTH_PORT=8080 \
    INTERNAL_PORT=8081 \
    LOG_FORMAT=json
USER 1000:1000
ENTRYPOINT ["/app/trivy-collector-scraper"]
