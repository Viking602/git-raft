.PHONY: help build test cli-test guardrails fmt fmt-check install

BIN := git-raft

help:
	@printf '%s\n' \
		'Available targets:' \
		'  make build       - cargo build' \
		'  make test        - cargo test' \
		'  make cli-test    - cargo test --test cli' \
		'  make guardrails  - cargo test --test guardrails' \
		'  make fmt         - cargo fmt' \
		'  make fmt-check   - cargo fmt --check' \
		'  make install     - cargo install --path . --force'

build:
	cargo build

test:
	cargo test

cli-test:
	cargo test --test cli

guardrails:
	cargo test --test guardrails

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

install:
	cargo install --path . --force
