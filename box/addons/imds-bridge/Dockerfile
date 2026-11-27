FROM scratch

LABEL org.opencontainers.image.title="imds-bridge" \
      org.opencontainers.image.description="Sidecar that serves EKS Pod Identity credentials through an EC2 instance metadata endpoint on loopback" \
      org.opencontainers.image.version="0.1.0" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.base.name="scratch" \
      org.opencontainers.image.deprecated="false"

ARG TARGETARCH
COPY --chmod=755 imds-bridge-linux-${TARGETARCH} /app/imds-bridge
EXPOSE 9090
USER 65532:65532
ENTRYPOINT ["/app/imds-bridge"]
