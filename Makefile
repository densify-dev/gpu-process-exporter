# SPDX-License-Identifier: Apache-2.0

SHELL := /bin/bash

CARGO_DENY_VERSION ?= 0.21.0
CARGO ?= cargo
PARITY_CANDIDATE ?= target/debug/gpu-exporter

.PHONY: all tools lint build build-binary build-multiarch test build-docker-image parity clean help

all: test

tools:
	@installed_version="$$($(CARGO) deny --version 2>/dev/null | awk '{print $$2}')"; \
	if [ "$$installed_version" = "$(CARGO_DENY_VERSION)" ]; then \
		echo "cargo-deny $(CARGO_DENY_VERSION) is already installed."; \
	else \
		echo "Installing cargo-deny $(CARGO_DENY_VERSION)..."; \
		$(CARGO) install --locked cargo-deny@$(CARGO_DENY_VERSION); \
	fi

lint: tools
	@echo "Running rustfmt, clippy, cargo-deny, and the test map check..."
	@$(CARGO) fmt --all --check
	@$(CARGO) clippy --workspace --all-targets --locked -- -D warnings
	@$(CARGO) deny check advisories licenses
	@parity/check-test-map.sh

build: clean lint
	@echo "Building packages..."
	@$(CARGO) build --workspace --locked

build-binary: clean
	@TARGETARCHES=host ./build.sh

build-multiarch: clean lint
	@echo "Building multi-arch binaries..."
	@./build.sh

test: lint
	@echo "Running tests..."
	@$(CARGO) test --workspace --locked

build-docker-image: test
	@echo "Building Docker image..."
	@./build-docker-image.sh

parity:
	@parity/run.sh --candidate $(PARITY_CANDIDATE) $(PARITY_ARGS)

clean:
	@echo "Cleaning build output..."
	@rm -rf bin

help:
	@echo "Available targets:"
	@echo "  all                 Run test (default)"
	@echo "  tools               Install cargo-deny $(CARGO_DENY_VERSION)"
	@echo "  lint                Run rustfmt, clippy, cargo-deny, and parity/check-test-map.sh"
	@echo "  build               Run cargo build --workspace"
	@echo "  build-binary        Build the release executable for the host architecture"
	@echo "  build-multiarch     Run build.sh"
	@echo "  test                Run cargo test --workspace"
	@echo "  build-docker-image  Run build-docker-image.sh"
	@echo "  parity              Run the differential parity harness (PARITY_CANDIDATE, PARITY_ARGS)"
	@echo "  clean               Delete bin/"
	@echo "  help                Show this help"
