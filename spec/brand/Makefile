# OHD brand pipeline.
#
# Two independent sources of truth:
#   - `source/branding.svg` (Inkscape) → the logos. Colors are whatever
#     the designer authored; the pipeline never recolors them (that would
#     fight the author every time they re-export).
#   - `tokens.json` → the app palette (CSS variables + Compose constants).
#
# Run `make brand` after editing the SVG to regenerate every derived
# logo artifact:
#   - per-variant standalone SVGs in built/
#   - viewBox-tightened so each renders at its natural size
#   - raster PNGs at 256/512/1024 + favicon-sized 32/64
#   - dropped into the per-app trees (Connect, cord-web, landing)
#
# Run `make tokens` after editing tokens.json to regenerate the CSS
# variables and Compose constants so the apps stay in lockstep.
#
# Run from spec/brand/.

PYTHON  := python3
RSVG    := rsvg-convert
INKSCAPE := DISPLAY= inkscape

SRC     := source/branding.svg
TOKENS  := tokens.json
BUILT   := built
PNG_DIR := $(BUILT)/png
RASTER_SIZES := 32 64 256 512 1024

# Per-app drop targets. Each LOGO_DIR is created on demand; the guard in
# `install` tests a known-existing ANCESTOR (the app root) so a missing
# brand/ subfolder doesn't make the -d test fail on its own non-existence.
WEB_ROOT         := ../../cord/web
WEB_LOGO_DIR     := $(WEB_ROOT)/public/brand
LANDING_ROOT     := ../../landing
LANDING_LOGO_DIR := $(LANDING_ROOT)/public/brand
ANDROID_RES      := ../../connect/android/app/src/main/res
ANDROID_NODPI    := $(ANDROID_RES)/drawable-nodpi
WEB_TOKENS_CSS   := $(WEB_ROOT)/src/tokens.generated.css
ANDROID_COLORS_KT := ../../connect/android/app/src/main/java/com/ohd/connect/ui/theme/OhdColorsGenerated.kt

.PHONY: help brand svg tighten raster install tokens clean

help:
	@echo "targets:"
	@echo "  make brand     full logo pipeline: extract -> tighten -> raster -> install"
	@echo "  make svg       extract per-variant SVGs from source/branding.svg"
	@echo "  make tighten   shrink each viewBox to its drawing bbox (needs inkscape)"
	@echo "  make raster    render PNGs at $(RASTER_SIZES)px from built/*.svg"
	@echo "  make install   copy built artifacts into per-app trees"
	@echo "  make tokens    regenerate CSS + Compose constants from tokens.json"
	@echo "  make clean     remove built/"

# Logo pipeline only — colors come from the SVG as authored. `make tokens`
# is intentionally NOT part of this chain; the palette has its own source.
brand: svg tighten raster install

# ---- svg ------------------------------------------------------------------
# Walk source/branding.svg for every <g inkscape:label='*_logo_for_*_bg'>;
# emit one standalone SVG per match into built/.
svg: $(SRC) extract.py
	@$(PYTHON) extract.py $(SRC) $(BUILT)

# ---- tighten --------------------------------------------------------------
# Shrink each file's viewBox to its drawing bbox so the logo renders at its
# natural aspect ratio without empty padding. Uses Inkscape headless.
tighten: tighten.py
	@$(PYTHON) tighten.py $(BUILT)

# ---- raster ---------------------------------------------------------------
# Render each built SVG at every size in RASTER_SIZES. Output filenames
# follow `<base>-<size>.png` so the install step can pick exactly what each
# consumer wants.
raster:
	@mkdir -p $(PNG_DIR)
	@for svg in $(BUILT)/*.svg; do \
	  base=$$(basename $$svg .svg); \
	  for size in $(RASTER_SIZES); do \
	    out=$(PNG_DIR)/$$base-$$size.png; \
	    $(RSVG) -w $$size $$svg -o $$out 2>/dev/null && echo "  $$out"; \
	  done; \
	done

# ---- install --------------------------------------------------------------
# Drop the cleaned SVG + the most-used raster sizes into each consuming
# app's asset tree. Each target is created on demand; missing app dirs
# (because the app crate isn't checked out / present) skip silently.
install:
	@if [ -d $(WEB_ROOT) ]; then \
	  mkdir -p $(WEB_LOGO_DIR); \
	  cp $(BUILT)/*.svg $(WEB_LOGO_DIR)/; \
	  cp $(PNG_DIR)/*.png $(WEB_LOGO_DIR)/; \
	  echo "  installed -> $(WEB_LOGO_DIR)"; \
	fi
	@if [ -d $(LANDING_ROOT) ]; then \
	  mkdir -p $(LANDING_LOGO_DIR); \
	  cp $(BUILT)/*.svg $(LANDING_LOGO_DIR)/; \
	  cp $(PNG_DIR)/*.png $(LANDING_LOGO_DIR)/; \
	  echo "  installed -> $(LANDING_LOGO_DIR)"; \
	fi
	@if [ -d $(ANDROID_RES) ]; then \
	  mkdir -p $(ANDROID_NODPI); \
	  cp $(PNG_DIR)/ohd-on-white-1024.png      $(ANDROID_NODPI)/ohd_logo.png; \
	  cp $(PNG_DIR)/identity-on-white-1024.png $(ANDROID_NODPI)/ohd_identity_logo.png; \
	  cp $(PNG_DIR)/cord-on-white-1024.png     $(ANDROID_NODPI)/cord_logo.png; \
	  echo "  installed -> $(ANDROID_NODPI)"; \
	  echo "  (PNG in drawable-nodpi; convert to VectorDrawable XML in the"; \
	  echo "   logo-wiring task if crisper scaling is needed)"; \
	fi

# ---- tokens ---------------------------------------------------------------
# Regenerate CSS variables (cord-web) + Compose constants (Connect Android)
# from tokens.json. Outputs are committed alongside the apps; rerunning is
# idempotent.
tokens: $(TOKENS) tokens.py
	@$(PYTHON) tokens.py $(TOKENS) \
	  --css $(WEB_TOKENS_CSS) \
	  --kotlin $(ANDROID_COLORS_KT)

# ---- clean ----------------------------------------------------------------
clean:
	rm -rf $(BUILT)
