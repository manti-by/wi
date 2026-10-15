BINARY := wi
TARGET_DIR := target

.PHONY: build run test check fmt lint clippy release install smoke clean help

build: ## Build the debug binary
	cargo build

# `make run` exists to hand a listener to a client, so it asks for HTTP rather
# than the stdio default. Override with `make run ARGS="--transport stdio"`.
ARGS ?= --transport http

run: ## Serve the wiki at ./wiki/pages over HTTP (ARGS=... to override)
	cargo run -- $(ARGS)

test: ## Run the unit tests
	cargo test

fmt: ## Format the source
	cargo fmt

check: ## Fail if the source is not formatted
	cargo fmt --check

lint: check clippy ## Format check and lint

clippy: ## Lint every target
	cargo clippy --all-targets

ci: check clippy test ## The gate CI runs before publishing anything

release: ## Build the release binary
	cargo build --release

install: ## Install the binary with cargo
	cargo install --path .

smoke: release ## Drive a real MCP session over both transports
	./scripts/smoke.sh

clean: ## Remove build artifacts
	cargo clean

help: ## List the targets
	@grep -hE '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-12s\033[0m %s\n", $$1, $$2}'
