CARGO ?= $(HOME)/.cargo/bin/cargo
PORT  ?= 8000

.DEFAULT_GOAL := run
.PHONY: run build start clean push seed

# Dev: Vite watch + cargo run concurrently. Both die on Ctrl+C.
run: frontend/node_modules dist/.vite/manifest.json
	@trap 'kill 0' EXIT INT TERM; \
	(cd frontend && bun run dev) & \
	PORT=$(PORT) $(CARGO) run

# Production build (Vite assets + release binary)
build: frontend/node_modules
	cd frontend && bun run build
	$(CARGO) build --release

# Run the release binary (after `make build`)
start:
	PORT=$(PORT) ./target/release/finance

# Wipe everything regenerable: build output, frontend deps, local data dir.
clean:
	rm -rf target dist frontend/node_modules data

push:
	git remote | xargs -I R git push R master

# Re-run the universe seed (curated symbols + bulk daily history). Idempotent.
seed:
	$(CARGO) run -- seed

frontend/node_modules:
	cd frontend && bun install

dist/.vite/manifest.json: frontend/node_modules
	cd frontend && bun run build
