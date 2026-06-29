.PHONY: all build build-debug test test-verbose clean lint fmt help

# Default target: show help
.DEFAULT_GOAL := help

BINARY_NAME := rpick
SRC_DIR := src
TARGET_DIR := target
INSTALL_DIR := $(HOME)/scripts
LLVM_PATH := /opt/homebrew/opt/llvm/lib

all: build

## Build the rpick binary and copy it to ~/scripts
build:
	@echo "  Building $(BINARY_NAME)..."
	DYLD_FALLBACK_LIBRARY_PATH="$(LLVM_PATH)" \
		cargo build --release 2>/dev/null || \
		cargo build --release
	@echo "  Copying to $(INSTALL_DIR)..."
	cp -f $(TARGET_DIR)/release/$(BINARY_NAME) $(INSTALL_DIR)/$(BINARY_NAME)
	@echo "  Done. Binary installed to $(INSTALL_DIR)/$(BINARY_NAME)"

## Build in debug mode (faster iteration)
build-debug:
	DYLD_FALLBACK_LIBRARY_PATH="$(LLVM_PATH)" cargo build

## Run all tests
test:
	DYLD_FALLBACK_LIBRARY_PATH="$(LLVM_PATH)" cargo test

## Run tests with output
test-verbose:
	DYLD_FALLBACK_LIBRARY_PATH="$(LLVM_PATH)" cargo test -- --nocapture

## Clean build artifacts
clean:
	cargo clean
	rm -f $(INSTALL_DIR)/$(BINARY_NAME)

## Run the linter (clippy)
lint:
	DYLD_FALLBACK_LIBRARY_PATH="$(LLVM_PATH)" cargo clippy -- -D warnings

## Format source code
fmt:
	cargo fmt

## Show this help message
help:
	@echo "  $(BINARY_NAME) - Video sorting utility (Rust port of gopick)"
	@echo ""
	@echo "  Usage: make <target>"
	@echo ""
	@echo "  Targets:"
	@echo "    all             Build release + install (default)"
	@echo "    build           Build release + install to ~/scripts"
	@echo "    build-debug     Build debug (no install)"
	@echo "    test            Run tests"
	@echo "    test-verbose    Run tests with stdout"
	@echo "    clean           Clean build artifacts"
	@echo "    lint            Run clippy"
	@echo "    fmt             Format source code"
	@echo "    help            Show this help"
	@echo ""
	@echo "  Example: make build"
