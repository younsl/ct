FROM public.ecr.aws/amazonlinux/amazonlinux:2023-minimal
ARG ARCH=aarch64
ARG BASE_URL=https://downloads.aws.com/aws-emulator/packages/aws-emulator/LTS/latest
ARG KEY_URL=https://downloads.aws.com/keys/gpg/current.asc
ARG PUBLISHER_FINGERPRINT=98BCF336A3BE53B734E967C52E5BF49BD940F624
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
WORKDIR /tmp/aws-emulator
RUN curl -fsSL -o aws-emulator.rpm "${BASE_URL}/aws-emulator-${ARCH}.rpm" \
    && curl -fsSL -o aws-emulator.rpm.sig "${BASE_URL}/aws-emulator-${ARCH}.rpm.sig" \
    && curl -fsSL -o aws-publishing.asc "${KEY_URL}" \
    && gpg --batch --with-colons --show-keys aws-publishing.asc | grep -q "^fpr:::::::::${PUBLISHER_FINGERPRINT}:" \
    && gpg --batch --dearmor -o aws-publishing.gpg aws-publishing.asc \
    && gpg --batch --no-default-keyring --keyring ./aws-publishing.gpg --verify aws-emulator.rpm.sig aws-emulator.rpm \
    && rpm -i aws-emulator.rpm \
    && rm -rf /tmp/aws-emulator /root/.gnupg \
    && mkdir -p /home/emulator && chown 1000 /home/emulator
WORKDIR /home/emulator
USER 1000
ENV HOME=/home/emulator EMULATOR_BIND_ADDRESS=0.0.0.0
ENTRYPOINT ["aws-emulator"]
