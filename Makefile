.DEFAULT_GOAL := help

.PHONY: help build test test-all-features test-nextest clippy fmt fmt-check lint-fix watch install-hooks deploy-testnet deploy-local bench clean

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2}'

build: ## Build all contracts
	cargo build --release

test: ## Run unit tests
	cargo test

test-all-features: ## Run unit tests with all features enabled
	cargo test --all-features

test-nextest: ## Run unit tests via cargo-nextest (requires cargo-nextest)
	cargo nextest run --workspace

clippy: ## Run Clippy lints
	cargo clippy --all-targets --all-features -- -D warnings

fmt: ## Format source code
	cargo fmt --all

fmt-check: ## Verify all Rust files are rustfmt-clean (exits non-zero on failure)
	./scripts/format-check.sh

lint-fix: ## Auto-fix formatting and common Clippy issues
	./scripts/lint-fix.sh

watch: ## Re-run tests on any src/ change (requires cargo-watch)
	cargo watch -w src -x test

install-hooks: ## Install git pre-commit hook (format-check + clippy)
	./scripts/install-hooks.sh

deploy-testnet: ## Deploy contracts to testnet
	./scripts/deploy.sh testnet

deploy-local: ## Deploy contracts to local node
	./scripts/deploy.sh local

bench: ## Run benchmarks
	cargo bench

new-contract: ## Scaffold a new contract: make new-contract NAME=my-contract
	@test -n "$(NAME)" || (echo "Usage: make new-contract NAME=<kebab-case-name>" && exit 1)
	./scripts/new-contract.sh $(NAME)

clean: ## Remove build artifacts
	cargo clean
