TOOLS := tools-base move-suggest move-bounds-checker move1-to-move2 aptos-gas-analyzer
# Tools that additionally require a local aptos-core checkout
APTOS_TOOLS := move-bounds-checker-native named-address-recover
ALL_TOOLS := $(TOOLS) $(APTOS_TOOLS)
# Tools that produce binaries (excludes libraries)
BIN_TOOLS := $(filter-out tools-base,$(TOOLS))
DIST_DIR := dist

.PHONY: all build build-all release fmt fmt-check lint clippy check test clean clean-all ci help package

all: build

help:
	@echo "Usage: make [target]"
	@echo ""
	@echo "Build:"
	@echo "  build       Build all tools (default)"
	@echo "  build-all   Build all tools including aptos-core dependents"
	@echo "  release     Build all tools in release mode"
	@echo "  check       Type-check all tools without building"
	@echo "  package     Build release binaries and zip them up"
	@echo "  clean       Remove build artifacts for CI tools"
	@echo "  clean-all   Remove build artifacts for all tools (needs aptos-core)"
	@echo ""
	@echo "Quality:"
	@echo "  lint        Run clippy and format check"
	@echo "  clippy      Run clippy lints"
	@echo "  fmt         Auto-format all tools"
	@echo "  fmt-check   Check formatting without modifying"
	@echo "  test        Run tests for all tools"
	@echo ""
	@echo "CI:"
	@echo "  ci          Run build, lint, and test"
	@echo ""
	@echo "Tools:       $(TOOLS)"
	@echo "Aptos tools: $(APTOS_TOOLS)"

ci: build lint test

build:
	@for tool in $(TOOLS); do \
		echo "Building $$tool..."; \
		cargo build --manifest-path $$tool/Cargo.toml || exit 1; \
	done

build-all:
	@for tool in $(ALL_TOOLS); do \
		echo "Building $$tool..."; \
		cargo build --manifest-path $$tool/Cargo.toml || exit 1; \
	done

release:
	@for tool in $(TOOLS); do \
		echo "Building $$tool (release)..."; \
		cargo build --release --manifest-path $$tool/Cargo.toml || exit 1; \
	done

fmt:
	@for tool in $(TOOLS); do \
		echo "Formatting $$tool..."; \
		cargo fmt --manifest-path $$tool/Cargo.toml || exit 1; \
	done

fmt-check:
	@for tool in $(TOOLS); do \
		echo "Checking format for $$tool..."; \
		cargo fmt --manifest-path $$tool/Cargo.toml -- --check || exit 1; \
	done

lint: clippy fmt-check

clippy:
	@for tool in $(TOOLS); do \
		echo "Linting $$tool..."; \
		cargo clippy --manifest-path $$tool/Cargo.toml -- -D warnings || exit 1; \
	done

check:
	@for tool in $(TOOLS); do \
		echo "Checking $$tool..."; \
		cargo check --manifest-path $$tool/Cargo.toml || exit 1; \
	done

test:
	@for tool in $(TOOLS); do \
		echo "Testing $$tool..."; \
		cargo test --manifest-path $$tool/Cargo.toml || exit 1; \
	done

package: release
	@rm -rf $(DIST_DIR)
	@mkdir -p $(DIST_DIR)
	@for tool in $(BIN_TOOLS); do \
		cp $$tool/target/release/$$tool $(DIST_DIR)/ || exit 1; \
	done
	@cd $(DIST_DIR) && zip -r ../aptos-move-tools.zip . && cd ..
	@rm -rf $(DIST_DIR)
	@echo "Packaged to aptos-move-tools.zip"

clean:
	@for tool in $(TOOLS); do \
		echo "Cleaning $$tool..."; \
		cargo clean --manifest-path $$tool/Cargo.toml; \
	done
	@rm -rf $(DIST_DIR) aptos-move-tools.zip

clean-all:
	@for tool in $(ALL_TOOLS); do \
		echo "Cleaning $$tool..."; \
		cargo clean --manifest-path $$tool/Cargo.toml; \
	done
	@rm -rf $(DIST_DIR) aptos-move-tools.zip
