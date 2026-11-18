.PHONY: help build release run dev dev-scraper dev-all export-state crd test fmt lint check clean install docker-build docker-push podman-build deps build-all frontend frontend-install frontend-dev

SCRAPER := trivy-collector-scraper
SERVER := trivy-collector-server
ECR_REGISTRY := <YOUR_ECR_REGISTRY>
VERSION := $(shell grep '^version' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
GIT_COMMIT := $(shell git rev-parse --short HEAD 2>/dev/null || echo "unknown")
BUILD_DATE := $(shell date -u +"%Y-%m-%dT%H:%M:%SZ")
PLATFORMS := linux/amd64,linux/arm64

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-20s\033[0m %s\n", $$1, $$2}'

build: ## Build debug binaries
	cargo build

release: ## Build optimized release binaries
	cargo build --release

# Shared internal-API token for local runs. Both processes must agree on it or
# the scraper rejects every request from the server.
DEV_INTERNAL_TOKEN := dev-internal-token

run: build ## Build and run the server (needs a scraper on :8081)
	SCRAPER_URL=http://localhost:8081 \
	INTERNAL_TOKEN=$(DEV_INTERNAL_TOKEN) \
	HUB_SECRET_NAMESPACE=trivy-system \
	LOG_FORMAT=pretty \
	./target/debug/$(SERVER)

dev: ## Run the server with debug logging (UI only, needs dev-scraper in another shell)
	SCRAPER_URL=http://localhost:8081 \
	INTERNAL_TOKEN=$(DEV_INTERNAL_TOKEN) \
	HUB_SECRET_NAMESPACE=trivy-system \
	EXTERNAL_URL=http://localhost:3000 \
	LOG_FORMAT=pretty \
	LOG_LEVEL=debug \
	cargo run --bin $(SERVER)

dev-scraper: ## Run scraper with debug logging (watchers, database, internal API :8081)
	STORAGE_PATH=/tmp/trivy-data \
	INTERNAL_TOKEN=$(DEV_INTERNAL_TOKEN) \
	INTERNAL_PORT=8081 \
	HUB_SECRET_NAMESPACE=trivy-system \
	HEALTH_PORT=8090 \
	LOG_FORMAT=pretty \
	LOG_LEVEL=debug \
	cargo run --bin $(SCRAPER)

dev-all: ## Run server (UI :3000) + scraper (internal API :8081) in one process group; Ctrl+C stops both
	@echo "==> Building once"
	@cargo build
	@echo "==> Starting server (UI :3000) + scraper (internal API :8081, health :8090), Ctrl+C stops both"
	@trap 'kill 0' INT TERM EXIT; \
	{ \
	  STORAGE_PATH=/tmp/trivy-data \
	  INTERNAL_TOKEN=$(DEV_INTERNAL_TOKEN) \
	  INTERNAL_PORT=8081 \
	  HUB_SECRET_NAMESPACE=trivy-system \
	  HEALTH_PORT=8090 \
	  LOG_FORMAT=pretty LOG_LEVEL=debug \
	  ./target/debug/$(SCRAPER) 2>&1 \
	    | sed -u 's/^/[scraper] /' ; \
	} & \
	{ \
	  SCRAPER_URL=http://localhost:8081 \
	  INTERNAL_TOKEN=$(DEV_INTERNAL_TOKEN) \
	  HUB_SECRET_NAMESPACE=trivy-system \
	  EXTERNAL_URL=http://localhost:3000 \
	  LOG_FORMAT=pretty LOG_LEVEL=debug \
	  ./target/debug/$(SERVER) 2>&1 \
	    | sed -u 's/^/[server ] /' ; \
	} & \
	wait

export-state: build ## Migrate authored state out of a legacy database (DB_PATH=/path/to/trivy.db)
	./target/debug/$(SCRAPER) export-state --db-path=$(or $(DB_PATH),/tmp/trivy-data/trivy.db) --dry-run

crd: ## Regenerate the AlertRule CRD in the Helm chart from crates/alertrule-crd/src/crd.rs
	cargo run --quiet --bin crdgen

test: ## Run tests
	cargo test --verbose

fmt: ## Format code
	cargo fmt

lint: ## Run clippy
	cargo clippy -- -D warnings

check: ## Check code without building
	cargo check

clean: ## Remove build artifacts
	cargo clean
	rm -rf /tmp/trivy-data

install: release ## Install both binaries to ~/.cargo/bin/
	cp target/release/$(SCRAPER) target/release/$(SERVER) ~/.cargo/bin/

deps: ## Update dependencies
	cargo update

docker-build: ## Build both images for the native arch (cross-compiles with cargo-zigbuild, frontend from `make frontend`)
	@ARCH=$$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/'); \
	if [ "$$ARCH" = "amd64" ]; then TARGET=x86_64-unknown-linux-musl; else TARGET=aarch64-unknown-linux-musl; fi; \
	for c in $(SCRAPER) $(SERVER); do \
	  dir=crates/$${c#trivy-collector-}; \
	  echo "Building $$c for $$TARGET with cargo-zigbuild..."; \
	  cargo zigbuild --release --target $$TARGET --bin $$c || exit 1; \
	  cp target/$$TARGET/release/$$c $$dir/$$c-linux-$$ARCH; \
	  docker build -t $$c:$(VERSION) -t $$c:latest $$dir || exit 1; \
	  rm -f $$dir/$$c-linux-$$ARCH; \
	done

docker-push: ## Push both images to registry
	for c in $(SCRAPER) $(SERVER); do \
	  docker tag $$c:$(VERSION) $(ECR_REGISTRY)/$$c:$(VERSION); \
	  docker tag $$c:latest $(ECR_REGISTRY)/$$c:latest; \
	  docker push $(ECR_REGISTRY)/$$c:$(VERSION); \
	  docker push $(ECR_REGISTRY)/$$c:latest; \
	done

podman-build: ## Build both images with Podman for the native arch (cross-compiles with cargo-zigbuild)
	@ARCH=$$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/'); \
	if [ "$$ARCH" = "amd64" ]; then TARGET=x86_64-unknown-linux-musl; else TARGET=aarch64-unknown-linux-musl; fi; \
	for c in $(SCRAPER) $(SERVER); do \
	  dir=crates/$${c#trivy-collector-}; \
	  echo "Building $$c for $$TARGET with cargo-zigbuild..."; \
	  cargo zigbuild --release --target $$TARGET --bin $$c || exit 1; \
	  cp target/$$TARGET/release/$$c $$dir/$$c-linux-$$ARCH; \
	  podman build -t $$c:$(VERSION) -t $$c:latest $$dir || exit 1; \
	  rm -f $$dir/$$c-linux-$$ARCH; \
	done

build-all: ## Build both binaries for all platforms (requires cargo-zigbuild)
	cargo zigbuild --release --target x86_64-unknown-linux-musl --bin $(SCRAPER) --bin $(SERVER)
	cargo zigbuild --release --target aarch64-unknown-linux-musl --bin $(SCRAPER) --bin $(SERVER)

frontend-install: ## Install frontend dependencies
	cd frontend && npm install

frontend: frontend-install ## Build frontend (outputs to static/)
	cd frontend && npm run build

frontend-dev: frontend-install ## Start frontend dev server with HMR
	cd frontend && npm run dev
