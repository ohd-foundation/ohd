# OHD Connect — build + release automation.
#
#   make build     cross-compile the Rust core + assemble the debug APK
#   make apk       assemble the debug APK only (skip the Rust rebuild)
#   make install   adb-install the APK on every connected device
#   make bump      versionCode +1, versionName -> beta<N+1>
#   make release   build, then publish the APK + refresh the landing page
#   make ship      bump + release in one shot
#
# Run from connect/android/. The Rust step needs the per-user rustup
# toolchain (system /usr/bin/cargo has no Android targets) — hence the
# explicit PATH prepend.

HOST      := root@178.105.71.238
APK       := app/build/outputs/apk/debug/app-debug.apk
GRADLE    := build.gradle.kts
LANDING   := ../../landing/index.html
BINDINGS  := ../../storage/crates/ohd-storage-bindings
JNILIBS   := app/src/main/jniLibs
CARGO_BIN := $(HOME)/.cargo/bin
REMOTE_APK     := /var/www/ohd-downloads/ohd-connect-latest.apk
REMOTE_LANDING := /opt/ohd/landing/index.html
RSYNC     := rsync -avz -e 'ssh -o BatchMode=yes'

.PHONY: help native apk build install bump release ship

help:
	@echo "targets: native apk build install bump release ship"
	@grep -E '^#   make' Makefile | sed 's/^#  //'

# Cross-compile ohd-storage-bindings for every Android ABI. Needed whenever
# the Rust core or a SQL migration changes.
native:
	cd $(BINDINGS) && PATH="$(CARGO_BIN):$$PATH" cargo ndk \
		-t arm64-v8a -t armeabi-v7a -t x86_64 \
		-o ../../../connect/android/$(JNILIBS) build --release

# Assemble the debug APK. Assumes the jniLibs/*.so are current.
apk:
	./gradlew :app:assembleDebug

# Full build: Rust core + APK.
build: native apk

# Install on every connected adb device (phone + emulator).
install:
	@for d in $$(adb devices | awk 'NR>1 && $$2=="device" {print $$1}'); do \
		echo "installing on $$d"; \
		adb -s $$d install -r $(APK); \
	done

# versionCode +1, versionName -> 0.1.0-beta<versionCode>.
bump:
	@CODE=$$(grep -oP 'versionCode = \K[0-9]+' app/$(GRADLE)); \
	NEW=$$((CODE + 1)); \
	sed -i -E \
		-e "s/versionCode = [0-9]+/versionCode = $$NEW/" \
		-e "s/versionName = \"0\.1\.0-beta[0-9]+\"/versionName = \"0.1.0-beta$$NEW\"/" \
		app/$(GRADLE); \
	echo "bumped to 0.1.0-beta$$NEW"

# Build, publish the APK to the download host, and rewrite the landing
# page's download card (version / size / sha256) so the three stay in sync.
release: build
	@VER=$$(grep -oP 'versionName = "\K[^"]+' app/$(GRADLE)); \
	SHA=$$(sha256sum $(APK) | cut -d' ' -f1); \
	SZ=$$(( $$(stat -c%s $(APK)) / 1024 / 1024 )); \
	echo "publishing $$VER — $$SZ MB — $$SHA"; \
	sed -i -E \
		-e "s#<code>0\.1\.0-beta[0-9]+ \(debug\)</code>#<code>$$VER (debug)</code>#" \
		-e "s#(<dt>Size</dt><dd>)<code>[0-9]+ MB</code>#\1<code>$$SZ MB</code>#" \
		-e "s#(<dt>SHA-256</dt><dd>)<code>[0-9a-f]+</code>#\1<code>$$SHA</code>#" \
		$(LANDING); \
	$(RSYNC) $(APK) $(HOST):$(REMOTE_APK); \
	$(RSYNC) $(LANDING) $(HOST):$(REMOTE_LANDING); \
	echo "live: https://ohd.dev/downloads/ohd-connect-latest.apk ($$VER)"

# Bump the version, then build + publish.
ship: bump release
