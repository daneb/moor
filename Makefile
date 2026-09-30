# moor dev tooling — mirrors .github/workflows/ci.yml so `make ci`
# runs the same checks locally that CI runs on push. See docs/CI.md for
# what each check catches and what's deliberately suppressed, and
# README.md's Security section for why. Targets that shell out to an
# external tool (shellcheck/hadolint/gitleaks/trivy) check it's on PATH
# first and say how to install it, rather than failing on a bare
# "command not found".

CLI_DIR := cli
DOCKERFILES := images/base/Dockerfile images/node/Dockerfile images/python/Dockerfile images/rust/Dockerfile proxy/Dockerfile
SHELL_SCRIPTS := images/build.sh proxy/entrypoint.sh tests/e2e.sh release.sh
IMAGES := base node rust python egress

.DEFAULT_GOAL := help

.PHONY: help
help: ## List available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

# --- build / clean -----------------------------------------------------

.PHONY: build
build: ## cargo build (debug)
	cd $(CLI_DIR) && cargo build

.PHONY: release
release: ## cargo build --release
	cd $(CLI_DIR) && cargo build --release

.PHONY: install
install: ## cargo install the CLI to ~/.cargo/bin
	cd $(CLI_DIR) && cargo install --path . --locked

.PHONY: clean
clean: ## cargo clean (does not touch Docker images/volumes — see `make images-clean`)
	cd $(CLI_DIR) && cargo clean

.PHONY: images
images: ## Build every moor Docker image (base, node, rust, python, egress)
	./images/build.sh

.PHONY: images-clean
images-clean: ## Remove every moor-tagged Docker image (does not touch project volumes)
	@for img in $(IMAGES); do docker rmi -f moor/$$img:latest 2>/dev/null || true; done

# --- format / lint / test ----------------------------------------------

.PHONY: fmt
fmt: ## cargo fmt (rewrites files)
	cd $(CLI_DIR) && cargo fmt

.PHONY: fmt-check
fmt-check: ## cargo fmt --check (fails if anything would be reformatted)
	cd $(CLI_DIR) && cargo fmt --check

.PHONY: lint
lint: ## cargo clippy --all-targets -D warnings
	cd $(CLI_DIR) && cargo clippy --all-targets -- -D warnings

.PHONY: test
test: ## cargo test
	cd $(CLI_DIR) && cargo test

# --- security scanning (each mirrors a CI job) --------------------------

.PHONY: audit
audit: ## cargo audit — RustSec advisory DB (needs: cargo install cargo-audit)
	@command -v cargo-audit >/dev/null 2>&1 || { echo "cargo-audit not found — install with: cargo install cargo-audit"; exit 1; }
	cd $(CLI_DIR) && cargo audit

.PHONY: deny
deny: ## cargo deny check — advisories/licenses/bans/sources (needs: cargo install cargo-deny)
	@command -v cargo-deny >/dev/null 2>&1 || { echo "cargo-deny not found — install with: cargo install cargo-deny"; exit 1; }
	cd $(CLI_DIR) && cargo deny check

.PHONY: shellcheck
shellcheck: ## shellcheck every .sh script (needs: brew install shellcheck)
	@command -v shellcheck >/dev/null 2>&1 || { echo "shellcheck not found — install with: brew install shellcheck"; exit 1; }
	shellcheck $(SHELL_SCRIPTS)

.PHONY: hadolint
hadolint: ## hadolint every Dockerfile, using .hadolint.yaml (needs: brew install hadolint)
	@command -v hadolint >/dev/null 2>&1 || { echo "hadolint not found — install with: brew install hadolint"; exit 1; }
	hadolint $(DOCKERFILES)

.PHONY: gitleaks
gitleaks: ## gitleaks over the full git history, using .gitleaks.toml (needs: brew install gitleaks)
	@command -v gitleaks >/dev/null 2>&1 || { echo "gitleaks not found — install with: brew install gitleaks"; exit 1; }
	gitleaks detect --source . --config .gitleaks.toml --no-banner

.PHONY: trivy
trivy: images ## Trivy CVE scan of every built image, fixable HIGH/CRITICAL only (needs: brew install trivy)
	@command -v trivy >/dev/null 2>&1 || { echo "trivy not found — install with: brew install trivy"; exit 1; }
	@for img in $(IMAGES); do \
		echo "== trivy scan: moor/$$img:latest =="; \
		trivy image --severity HIGH,CRITICAL --ignore-unfixed --exit-code 1 moor/$$img:latest || exit 1; \
	done

.PHONY: security
security: audit deny shellcheck hadolint gitleaks trivy ## Run every security scan (everything CI's security jobs run)

# --- end-to-end -----------------------------------------------------------

.PHONY: e2e
e2e: images ## Real end-to-end test against live Docker containers (tests/e2e.sh)
	bash tests/e2e.sh

# --- the full local CI replica -------------------------------------------

.PHONY: ci
ci: fmt-check lint test security e2e ## Everything .github/workflows/ci.yml runs, run locally in one shot

.PHONY: check
check: fmt-check lint test ## Fast checks only (no Docker, no external tools) — fmt/clippy/test

# --- release / publish ----------------------------------------------------

.PHONY: publish-dry-run
publish-dry-run: ## cargo publish --dry-run — validates packaging without publishing anything
	cd $(CLI_DIR) && cargo publish --dry-run

.PHONY: publish
publish: publish-dry-run ## cargo publish for real — irreversible (crates.io versions can be yanked, never deleted or reused). Runs the dry-run first, then asks for a typed confirmation. Requires `cargo login` already done.
	@echo ""
	@echo "publish-dry-run passed — see the \"Packaging ...\" line above for the exact name/version this will publish."
	@echo "This next step is real and cannot be undone."
	@read -p "Type 'publish' to continue: " confirm && [ "$$confirm" = "publish" ] || { echo "Aborted."; exit 1; }
	cd $(CLI_DIR) && cargo publish
