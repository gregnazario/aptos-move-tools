TOOLS := tools-base move-suggest move-bounds-checker move-bounds-checker-native move1-to-move2 named-address-recover
# Tools that build without aptos-core path deps (for CI)
CI_TOOLS := tools-base move-suggest move-bounds-checker move1-to-move2

.PHONY: all build release fmt lint clippy check test clean ci

all: build

ci: build-ci lint-ci test-ci

build-ci:
	@for tool in $(CI_TOOLS); do \
		echo "Building $$tool..."; \
		cargo build --manifest-path $$tool/Cargo.toml; \
	done

lint-ci:
	@for tool in $(CI_TOOLS); do \
		echo "Linting $$tool..."; \
		cargo clippy --manifest-path $$tool/Cargo.toml -- -D warnings; \
		cargo fmt --manifest-path $$tool/Cargo.toml -- --check; \
	done

test-ci:
	@for tool in $(CI_TOOLS); do \
		echo "Testing $$tool..."; \
		cargo test --manifest-path $$tool/Cargo.toml; \
	done

build:
	@for tool in $(TOOLS); do \
		echo "Building $$tool..."; \
		cargo build --manifest-path $$tool/Cargo.toml; \
	done

release:
	@for tool in $(TOOLS); do \
		echo "Building $$tool (release)..."; \
		cargo build --release --manifest-path $$tool/Cargo.toml; \
	done

fmt:
	@for tool in $(TOOLS); do \
		echo "Formatting $$tool..."; \
		cargo fmt --manifest-path $$tool/Cargo.toml; \
	done

fmt-check:
	@for tool in $(TOOLS); do \
		echo "Checking format for $$tool..."; \
		cargo fmt --manifest-path $$tool/Cargo.toml -- --check; \
	done

lint: clippy fmt-check

clippy:
	@for tool in $(TOOLS); do \
		echo "Linting $$tool..."; \
		cargo clippy --manifest-path $$tool/Cargo.toml -- -D warnings; \
	done

check:
	@for tool in $(TOOLS); do \
		echo "Checking $$tool..."; \
		cargo check --manifest-path $$tool/Cargo.toml; \
	done

test:
	@for tool in $(TOOLS); do \
		echo "Testing $$tool..."; \
		cargo test --manifest-path $$tool/Cargo.toml; \
	done

clean:
	@for tool in $(TOOLS); do \
		echo "Cleaning $$tool..."; \
		cargo clean --manifest-path $$tool/Cargo.toml; \
	done
