# SPDX-License-Identifier: Apache-2.0

SHELL := /bin/bash

LINTER_VERSION ?= v2.13.1
GO ?= go
GOPATH ?= $(shell $(GO) env GOPATH)
GOLANGCI_LINT ?= $(GOPATH)/bin/golangci-lint

.PHONY: all tools lint build build-binary build-multiarch test build-docker-image clean help

all: test

tools:
	@installed_version=""; \
	if [ -x "$(GOLANGCI_LINT)" ]; then \
		installed_version="v$$("$(GOLANGCI_LINT)" --version | awk '{print $$4}')"; \
	fi; \
	if [ "$$installed_version" = "$(LINTER_VERSION)" ]; then \
		echo "golangci-lint $(LINTER_VERSION) is already installed."; \
	else \
		echo "Installing golangci-lint $(LINTER_VERSION)..."; \
		$(GO) install github.com/golangci/golangci-lint/v2/cmd/golangci-lint@$(LINTER_VERSION); \
	fi

lint: tools
	@echo "Running golangci-lint..."
	@$(GOLANGCI_LINT) run ./...

build: clean lint
	@echo "Building packages..."
	@$(GO) build ./...

build-binary: clean
	@case "$$(uname -m)" in \
		x86_64|amd64) TARGETARCH=amd64 ;; \
		aarch64|arm64) TARGETARCH=arm64 ;; \
		*) echo "Unsupported host architecture: $$(uname -m)" >&2; exit 1 ;; \
	esac; \
	mkdir -p bin/$$TARGETARCH; \
	echo "Building linux/$$TARGETARCH executable..."; \
	GOOS=linux \
	GOARCH=$$TARGETARCH \
	CGO_ENABLED=1 \
	CGO_CFLAGS="-Wno-deprecated-declarations" \
	CC=gcc \
	$(GO) build -trimpath \
	-ldflags="-w -s" \
	-o bin/$$TARGETARCH/gpu-exporter cmd/main.go

build-multiarch: clean lint
	@echo "Building multi-arch binaries..."
	@./build.sh

test: lint
	@echo "Running tests..."
	@$(GO) test ./...
	@$(GO) test -race ./...

build-docker-image: test
	@echo "Building Docker image..."
	@./build-docker-image.sh

clean:
	@echo "Cleaning build output..."
	@rm -rf bin

help:
	@echo "Available targets:"
	@echo "  all                 Run test (default)"
	@echo "  tools               Install golangci-lint $(LINTER_VERSION)"
	@echo "  lint                Run golangci-lint"
	@echo "  build               Run go build ./..."
	@echo "  build-binary        Build linux executable for the runner architecture"
	@echo "  build-multiarch     Run build.sh"
	@echo "  test                Run go test and go test -race"
	@echo "  build-docker-image  Run build-docker-image.sh"
	@echo "  clean               Delete bin/"
	@echo "  help                Show this help"
