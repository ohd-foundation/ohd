# OHD Backend — local build + ship to the Hetzner host.
#
#   make build       build all server images locally (sequential, ~9 min cold)
#   make ship        build + save + scp + docker load + docker compose up -d
#   make logs        tail logs from every container
#   make ps          show running containers on the host
#   make bootstrap   first-time provisioning on a fresh Ubuntu/Debian host:
#                    install Docker, rsync code, generate /opt/ohd/.env if
#                    missing (prompts for OHD_CORD_ANTHROPIC_KEY via env
#                    var), full ship. Idempotent — safe to re-run.
#   make ssh         interactive shell on the host over IPv6
#   make nuke        WARNING — stop + remove every container + every named
#                    volume on the host (full data loss). Recreate-from-zero
#                    cycle. Re-run `make bootstrap` after.
#
# Run from deploy/host/. Most actions reach the host over IPv6 because
# IPv4 to Hetzner sometimes ends up in their automatic DDoS mitigation
# (see git history for details) and IPv6 is unaffected.

# IPv6 host literal. SSH wants `user@addr` with no brackets; rsync's URL
# parser wants `user@[addr]:path`. We keep both forms so each tool sees
# the syntax it expects, and use storage.ohd.dev (A-record only) as a
# stable identifier in messages.
HOST_SSH   := root@2a01:4f8:c014:d23b::1
HOST_RSYNC := root@[2a01:4f8:c014:d23b::1]
HOST_V4    := root@178.105.71.238
HOST_DNS   := root@storage.ohd.dev
SSH        := ssh -o BatchMode=yes -o ServerAliveInterval=30 -o ServerAliveCountMax=20
RSYNC      := rsync -az -e "$(SSH)" --exclude=target/ --exclude=node_modules/ \
              --exclude=.venv/ --exclude=__pycache__/ --exclude=build/ \
              --exclude=dist/ --exclude=.git/ --exclude='*.apk' \
              --exclude=cargo-target/

# Compose lives at deploy/host/docker-compose.yml; the build contexts
# climb `../..` to the repo root, so most docker commands run from
# there. Repo root = ../.. from this Makefile.
REPO     := ../..
COMPOSE  := docker compose -f deploy/host/docker-compose.yml \
            --profile cord --profile idp
SERVICES := ohd-storage ohd-relay ohd-saas ohd-idp ohd-cord
IMAGES   := $(SERVICES:%=%:dev)

.PHONY: help build ship logs ps bootstrap ssh nuke

help:
	@echo "targets:"
	@grep -E '^#   make' Makefile | sed 's/^#  //'

# ---- build ----------------------------------------------------------------
#
# Builds the five backend service images locally on the host's docker
# daemon. SEQUENTIAL — running cargo+apt in parallel across stages
# saturates the Debian mirror and apt-get update times out (real
# observed failure, 192 s deadline). Sequential is fast enough on a
# Ryzen-class host: ~9 min cold, ~30 s warm.
#
# Profile gating: ohd-cord and ohd-idp are profile-gated in compose, so
# their build targets need --profile.
build:
	@for svc in $(SERVICES); do \
		case $$svc in \
			ohd-cord) prof="--profile cord" ;; \
			ohd-idp)  prof="--profile idp"  ;; \
			*)        prof="" ;; \
		esac; \
		echo "=== build $$svc ==="; \
		(cd $(REPO) && docker compose -f deploy/host/docker-compose.yml $$prof \
			build --progress=plain $$svc) || exit $$?; \
	done
	@echo "=== images ==="
	@docker images --format '{{.Repository}}:{{.Tag}}\t{{.Size}}\t({{.CreatedSince}})' \
		| grep '^ohd-'

# ---- ship -----------------------------------------------------------------
#
# Build, then stream all five images through a single ssh+gzip pipe
# (`docker save | ssh ... docker load`), rsync the compose+caddy+landing
# bits, and `docker compose up -d` on the host with both profiles.
ship: build
	@echo "=== save + load over $(HOST_SSH) ==="
	docker save $(IMAGES) | gzip | $(SSH) $(HOST_SSH) 'gzip -d | docker load'
	@echo "=== rsync config + landing ==="
	$(RSYNC) $(REPO)/deploy/host/ $(HOST_RSYNC):/opt/ohd/deploy/host/
	$(RSYNC) $(REPO)/landing/ $(HOST_RSYNC):/opt/ohd/landing/
	$(RSYNC) $(REPO)/idp/idp.toml $(HOST_RSYNC):/opt/ohd/deploy/host/idp.toml
	@echo "=== docker compose up -d ==="
	$(SSH) $(HOST_SSH) 'cd /opt/ohd && $(COMPOSE) --env-file .env up -d'
	@$(MAKE) --no-print-directory ps

# ---- logs / ps / ssh ------------------------------------------------------
#
# Operational shims so the day-to-day "what's happening on the host"
# loop doesn't require remembering the compose path.

logs:
	$(SSH) $(HOST_SSH) 'cd /opt/ohd && $(COMPOSE) logs --tail=50 -f'

ps:
	$(SSH) $(HOST_SSH) 'cd /opt/ohd && $(COMPOSE) ps --format "table {{.Name}}\t{{.Status}}"'

ssh:
	@$(SSH) -t $(HOST_SSH)

# ---- bootstrap ------------------------------------------------------------
#
# Fresh-box provisioning. Run once after creating a new Hetzner cx33
# (or whatever) — it installs Docker, mirrors the code, writes the
# secrets file, and ships every image. Idempotent: Docker install
# is a no-op if already present; .env is preserved if it already
# exists (so re-running doesn't rotate secrets and lose your sessions).
#
# Required env var on first run:
#   OHD_CORD_ANTHROPIC_KEY  — your paid Anthropic API key (sk-ant-…).
#                             Only the Anthropic key has to come from you;
#                             everything else is generated locally as
#                             base64-of-32-bytes (cord/idp config decodes
#                             via base64; hex was the wrong shape and made
#                             cord crash-loop with "expected 32 bytes,
#                             got 48").
bootstrap:
	@if [ -z "$$OHD_CORD_ANTHROPIC_KEY" ]; then \
		if $(SSH) $(HOST_SSH) 'test -s /opt/ohd/.env' 2>/dev/null; then \
			echo "==> /opt/ohd/.env already present on host; reusing it"; \
		else \
			echo "ERROR: first-time bootstrap needs OHD_CORD_ANTHROPIC_KEY="; \
			echo "  (sk-ant-… — your paid Anthropic API key)"; \
			echo "  Re-run as: OHD_CORD_ANTHROPIC_KEY=sk-ant-… make bootstrap"; \
			exit 1; \
		fi; \
	fi
	@echo "=== docker install (idempotent) ==="
	$(SSH) $(HOST_SSH) bash -se < bootstrap-docker.sh
	@echo "=== mkdir /opt/ohd /var/www/ohd-downloads ==="
	$(SSH) $(HOST_SSH) 'mkdir -p /opt/ohd /var/www/ohd-downloads'
	@echo "=== rsync code ==="
	$(RSYNC) $(REPO)/ $(HOST_RSYNC):/opt/ohd/
	@echo "=== ensure /opt/ohd/.env exists ==="
	@if [ -n "$$OHD_CORD_ANTHROPIC_KEY" ]; then \
		echo "  writing /opt/ohd/.env with provided Anthropic key + fresh randoms"; \
		ANTHROPIC="$$OHD_CORD_ANTHROPIC_KEY"; \
		S_SESSION=$$(openssl rand -base64 32); \
		S_DATA=$$(openssl rand -base64 32); \
		S_OIDC=$$(openssl rand -base64 32); \
		S_SAAS=$$(openssl rand -base64 32); \
		printf 'OHD_CORD_SESSION_SECRET=%s\nOHD_CORD_DATA_KEY=%s\nOHD_CORD_OIDC_OHD_SECRET=%s\nOHD_IDP_CLIENT_CORD_SECRET=%s\nOHD_SAAS_JWT_SECRET=%s\nOHD_CORD_ANTHROPIC_KEY=%s\n' \
			"$$S_SESSION" "$$S_DATA" "$$S_OIDC" "$$S_OIDC" "$$S_SAAS" "$$ANTHROPIC" \
			| $(SSH) $(HOST_SSH) 'umask 077 && cat > /opt/ohd/.env && chmod 600 /opt/ohd/.env'; \
		unset ANTHROPIC S_SESSION S_DATA S_OIDC S_SAAS; \
	else \
		echo "  reusing existing /opt/ohd/.env"; \
	fi
	@$(MAKE) --no-print-directory ship

# ---- nuke -----------------------------------------------------------------
#
# Stop everything, remove every container and every named volume.
# Use only when you're certain — data loss is total.

nuke:
	@printf "About to destroy all containers + volumes on $(HOST_SSH). Confirm with: make nuke-confirm\n"

nuke-confirm:
	$(SSH) $(HOST_SSH) 'cd /opt/ohd && $(COMPOSE) down -v --remove-orphans'
	$(SSH) $(HOST_SSH) 'docker volume rm $$(docker volume ls -q | grep ^ohd_) 2>/dev/null || true'
	@echo "host wiped. re-run: OHD_CORD_ANTHROPIC_KEY=… make bootstrap"
