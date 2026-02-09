SHELL := /bin/bash

CARGO = cargo
CARGO_OPTS =
FMT_OPTS = 

CURRENT_DIR = $(shell pwd)
NAME := jaytripper

.PHONY: all pre-commit build clean version fmt fmt-check lint fix test sqlx-refresh sqlx-verify _sqlx-db-ready

SQLX_DATABASE_URL ?= sqlite://sqlx-dev.db

all: pre-commit build
pre-commit: lint fmt-check test

build:
	$(CARGO) $(CARGO_OPTS) build

clean:
	$(CARGO) $(CARGO_OPTS) clean

fmt: CARGO_OPTS += +nightly
fmt:
	$(CARGO) $(CARGO_OPTS) fmt --all -- $(FMT_OPTS)

fmt-check: FMT_OPTS += --check
fmt-check: fmt

fix:
	$(CARGO) $(CARGO_OPTS) fix --allow-staged
	$(CARGO) $(CARGO_OPTS) clippy --fix --allow-staged --allow-dirty

lint:
	$(CARGO) $(CARGO_OPTS) clippy --workspace --all-targets --all-features -- -D warnings

test:
	$(CARGO) $(CARGO_OPTS) test --all

_sqlx-db-ready:
	sqlx database create --database-url "$(SQLX_DATABASE_URL)"
	sqlx migrate run --database-url "$(SQLX_DATABASE_URL)" --source jaytripper_store/migrations

sqlx-refresh: _sqlx-db-ready
	DATABASE_URL="$(SQLX_DATABASE_URL)" $(CARGO) $(CARGO_OPTS) sqlx prepare --workspace

sqlx-verify: _sqlx-db-ready
	DATABASE_URL="$(SQLX_DATABASE_URL)" $(CARGO) $(CARGO_OPTS) sqlx prepare --workspace --check
	SQLX_OFFLINE=true $(CARGO) $(CARGO_OPTS) check -p jaytripper_store
