# Nucleus developer task runner.
# Thin wrappers over cargo so `make` and CI run the exact same checks.

.PHONY: all build test fmt fmt-check lint check ci clean

CARGO ?= cargo

# Default: the full local quality gate.
all: check

build:
	$(CARGO) build --workspace

test:
	$(CARGO) test --workspace

# Apply formatting in place.
fmt:
	$(CARGO) fmt --all

# Verify formatting without changing files (used by CI).
fmt-check:
	$(CARGO) fmt --all -- --check

# Lint with warnings treated as errors.
lint:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

# Everything CI runs, in order. Run this before pushing.
check: fmt-check lint test

ci: check

clean:
	$(CARGO) clean
