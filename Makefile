# Makefile : raccourcis de développement, d'audit et de release pour Hormos.
# Usage : make <cible>

CARGO ?= cargo
MUSL_TARGET := x86_64-unknown-linux-musl

# Scripts shell du dépôt (analysés par bash -n et ShellCheck).
SHELL_SCRIPTS := \
	scripts/package-release.sh \
	scripts/generate-sbom.sh \
	scripts/checksums-release.sh \
	scripts/sign-release.sh \
	scripts/intoto-provenance.sh \
	scripts/create-release-commit.sh \
	scripts/promote-release.sh \
	scripts/validate-artifacts.sh \
	scripts/publish-github-release.sh \
	scripts/test-sign-identity.sh \
	scripts/lib/release-lib.sh \
	scripts/tests/release-invariants.sh

.DEFAULT_GOAL := build
.PHONY: build release test lint fmt check audit security-check \
        shellcheck actionlint sast security-full release-check release-tests \
        sbom sign-check clean help

## build : compilation en mode debug
build:
	$(CARGO) build --workspace

## release : compilation optimisée
release:
	$(CARGO) build --workspace --release

## test : exécute la suite de tests
test:
	$(CARGO) test --workspace --locked

## lint : formatage (vérif) + clippy strict
lint:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets --all-features --locked -- -D warnings

## fmt : applique le formatage
fmt:
	$(CARGO) fmt --all

## check : contrôle rapide avant commit (fmt + clippy + tests)
check:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets --all-features --locked -- -D warnings
	$(CARGO) test --workspace --locked

## audit : outils DevSecOps (ignorés proprement si absents)
audit:
	@echo "==> Audit de sécurité et chaîne d'approvisionnement"
	@if command -v cargo-audit >/dev/null 2>&1; then \
		echo "--- cargo audit"; $(CARGO) audit; \
	else echo "!! cargo-audit absent - 'cargo install cargo-audit'"; fi
	@if command -v cargo-deny >/dev/null 2>&1; then \
		echo "--- cargo deny check"; $(CARGO) deny check; \
	else echo "!! cargo-deny absent - 'cargo install cargo-deny'"; fi
	@if command -v cargo-machete >/dev/null 2>&1; then \
		echo "--- cargo machete"; $(CARGO) machete; \
	else echo "!! cargo-machete absent - 'cargo install cargo-machete'"; fi
	@if command -v gitleaks >/dev/null 2>&1; then \
		echo "--- gitleaks detect"; gitleaks detect --source . --no-banner; \
	else echo "!! gitleaks absent - https://github.com/gitleaks/gitleaks"; fi

## security-check : alias de `audit`
security-check: audit

## shellcheck : analyse statique des scripts shell (ShellCheck requis)
shellcheck:
	@if ! command -v shellcheck >/dev/null 2>&1; then \
		echo "!! shellcheck absent - https://github.com/koalaman/shellcheck"; exit 1; \
	fi
	@echo "==> ShellCheck"
	shellcheck -x $(SHELL_SCRIPTS)

## actionlint : lint statique des workflows GitHub Actions (actionlint requis)
actionlint:
	@if ! command -v actionlint >/dev/null 2>&1; then \
		echo "!! actionlint absent - https://github.com/rhysd/actionlint"; exit 1; \
	fi
	@echo "==> actionlint"
	actionlint

## sast : analyse statique locale (clippy strict + shellcheck + actionlint)
# CodeQL (SAST Rust approfondi) tourne en CI via .github/workflows/codeql.yml.
sast:
	@echo "==> SAST local (clippy + shellcheck + actionlint)"
	$(CARGO) clippy --workspace --all-targets --all-features --locked -- -D warnings
	$(MAKE) shellcheck
	$(MAKE) actionlint

## security-full : porte de sécurité complète (audit supply chain + lint infra)
security-full:
	@echo "==> Security-full : audit supply chain + ShellCheck + actionlint"
	@if ! command -v cargo-audit >/dev/null 2>&1; then \
		echo "!! cargo-audit absent"; exit 1; fi
	@if ! command -v cargo-deny >/dev/null 2>&1; then \
		echo "!! cargo-deny absent"; exit 1; fi
	@if ! command -v cargo-machete >/dev/null 2>&1; then \
		echo "!! cargo-machete absent"; exit 1; fi
	@if ! command -v gitleaks >/dev/null 2>&1; then \
		echo "!! gitleaks absent"; exit 1; fi
	@echo "--- cargo audit"
	$(CARGO) audit
	@echo "--- cargo deny check"
	$(CARGO) deny check
	@echo "--- cargo machete"
	$(CARGO) machete
	@echo "--- gitleaks detect"
	gitleaks detect --source . --redact --verbose
	$(MAKE) shellcheck
	$(MAKE) actionlint

## release-check : porte de qualité complète avant release (syntaxe scripts incluse)
release-check:
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets --all-features --locked -- -D warnings
	$(CARGO) test --workspace --locked
	$(CARGO) build --workspace --release
	$(CARGO) build --workspace --release --target $(MUSL_TARGET)
	for s in $(SHELL_SCRIPTS); do bash -n "$$s"; done
	bash scripts/test-sign-identity.sh
	bash scripts/tests/release-invariants.sh

## release-tests : tests des invariants de release (SHA, no-op, worktree, tag, Sigstore)
release-tests:
	bash scripts/tests/release-invariants.sh

## sbom : génère le SBOM CycloneDX (nécessite cargo-cyclonedx épinglé)
sbom:
	@if ! cargo cyclonedx --version >/dev/null 2>&1; then \
		echo "!! cargo-cyclonedx absent - 'cargo install cargo-cyclonedx --version 0.5.9 --locked'"; exit 1; \
	fi
	bash scripts/generate-sbom.sh

## sign-check : vérifie l'outillage de signature + identité Sigstore (sans signer)
sign-check:
	@if command -v cosign >/dev/null 2>&1; then \
		echo "--- cosign disponible"; cosign version; \
	else echo "!! cosign absent - voir le workflow CI (job publish)"; fi
	bash -n scripts/sign-release.sh
	bash scripts/test-sign-identity.sh
	@echo "OK : la signature/provenance keyless réelle s'exécute uniquement en CI (OIDC)."

## clean : nettoie les artefacts de compilation
clean:
	$(CARGO) clean

## help : liste les cibles disponibles
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## /  /'
