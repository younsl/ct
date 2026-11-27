BINARY     := imds-bridge
VERSION    := $(shell grep -oE 'org\.opencontainers\.image\.version="[^"]+"' Dockerfile | cut -d'"' -f2)
REGISTRY   ?= ghcr.io/younsl
IMAGE      := $(REGISTRY)/$(BINARY)
COVER_MIN  ?= 70

.PHONY: all build release test emulator-test coverage fmt lint check clean \
        zigbuild-amd64 zigbuild-arm64 zigbuild docker-build docker-push help

.DEFAULT_GOAL := help

## all: fmt + lint + test + build
all: fmt lint test build

## build: compile debug binary
build:
	cargo build

## release: compile optimized release binary
release:
	cargo build --release

## test: run tests
test:
	cargo test

## emulator-test: push and pull through a Harbor registry that reaches emulated S3 via imds-bridge (docker or podman)
emulator-test:
	./tests/aws-emulator/run.sh

## coverage: enforce minimum line coverage (cargo-llvm-cov)
coverage:
	cargo llvm-cov --fail-under-lines $(COVER_MIN)

## fmt: format code
fmt:
	cargo fmt

## lint: rustfmt check + clippy with warnings denied
lint:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

## check: type-check without building
check:
	cargo check

## clean: remove build artifacts
clean:
	cargo clean
	rm -f $(BINARY)-linux-*

## zigbuild-amd64: cross-compile static linux/amd64 binary
zigbuild-amd64:
	cargo zigbuild --release --target x86_64-unknown-linux-musl
	cp target/x86_64-unknown-linux-musl/release/$(BINARY) $(BINARY)-linux-amd64

## zigbuild-arm64: cross-compile static linux/arm64 binary
zigbuild-arm64:
	cargo zigbuild --release --target aarch64-unknown-linux-musl
	cp target/aarch64-unknown-linux-musl/release/$(BINARY) $(BINARY)-linux-arm64

## zigbuild: cross-compile both architectures
zigbuild: zigbuild-amd64 zigbuild-arm64

## docker-build: cross-compile both arches, then build multi-arch image (requires buildx)
docker-build: zigbuild
	docker buildx build --platform linux/amd64,linux/arm64 \
		-t $(IMAGE):$(VERSION) -t $(IMAGE):latest .

## docker-push: cross-compile both arches, then build and push multi-arch image
docker-push: zigbuild
	docker buildx build --platform linux/amd64,linux/arm64 \
		-t $(IMAGE):$(VERSION) -t $(IMAGE):latest --push .

## help: show this message
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/^## //' | column -t -s ':'
