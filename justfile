# Personal recipes for this fork (fullerzz/hex, macOS only).
#
# This fork never downloads speech models: every model file, the Silero
# speech detector included, is one you supply (docs/features/own-models.md).
# The model recipes below are for those files. Nothing here touches OpenCode.
#
# Builds are ad-hoc signed (HEX_RELEASE_UNSIGNED=1), because there is no
# Developer ID in the keychain, and marked local (HEX_LOCAL_BUILD=1), so the
# app never updates itself back to upstream's release. An ad-hoc signature
# changes with every build, so macOS may drop the Accessibility grant after
# each install: run `just reset-accessibility`, then grant it again.

set positional-arguments
set shell := ["bash", "-euo", "pipefail", "-c"]

# The installed app's CLI. `models`, `use`, `k-quant`, and `status` talk to the
# running Hex; `import`, `import-detector`, and `remove` run on their own.
# Point HEX_BIN at target/app/Hex.app/Contents/MacOS/hex to use a fresh build.
hex := env("HEX_BIN", "/Applications/Hex.app/Contents/MacOS/hex")
upstream_url := "https://github.com/anomalyco/hex.git"
build_env := "HEX_RELEASE_UNSIGNED=1 HEX_LOCAL_BUILD=1"

# List recipes.
default:
    @just --list --unsorted

# --- Build and install -------------------------------------------------------

# Build target/app/Hex.app from this checkout.
build:
    {{ build_env }} scripts/build-app.sh

# Build, stop the running Hex, replace /Applications/Hex.app, and open it.
install:
    {{ build_env }} scripts/install-app.sh

# fmt, clippy, and tests (optionally only those matching a filter).
check filter="":
    scripts/check {{ filter }}

# Clear the stale Accessibility grant a reinstall leaves behind.
reset-accessibility:
    tccutil reset Accessibility anomaly.hex

# Ask the running Hex for its state.
status:
    "{{ hex }}" status

# --- Your model files --------------------------------------------------------

# Every model, where it comes from, and whether it is on this computer.
models:
    "{{ hex }}" model list

# Check and copy in a GGUF speech model; prints its custom-… id.
import path name="":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "${2:-}" ]; then
      "{{ hex }}" model import "$1" --name "$2"
    else
      "{{ hex }}" model import "$1"
    fi

# Install the Silero VAD v6.2 speech detector from a file you supply.
import-detector path:
    "{{ hex }}" model import --speech-detector "$1"

# Switch the running Hex to an installed model.
use id language="":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "${2:-}" ]; then
      "{{ hex }}" model use "$1" --language "$2"
    else
      "{{ hex }}" model use "$1"
    fi

# Remove Hex's copy of an import (never the selected model).
remove id:
    "{{ hex }}" model remove "$1"

# Allow Q6_K, Q5_K, and Q4_K model files (Labs › Smaller model files).
k-quant on="on":
    "{{ hex }}" labs "$1" k-quant-models

# Open Hex's models folder in Finder.
models-dir:
    open "$HOME/Library/Application Support/hex/models"

# --- Upstream ----------------------------------------------------------------

# Fast-forward dev to upstream's dev, then rebase the current branch onto it.
sync:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
      echo "Working tree has changes; commit or stash them first." >&2
      exit 1
    fi
    git remote get-url upstream >/dev/null 2>&1 || git remote add upstream "{{ upstream_url }}"
    git fetch upstream dev
    branch=$(git branch --show-current)
    if [ "$branch" = dev ]; then
      git merge --ff-only upstream/dev
      exit 0
    fi
    git fetch . upstream/dev:dev
    git rebase dev
    echo "Rebased $branch onto dev ($(git rev-parse --short dev)). Run 'just push' to publish."

# Push dev and the current branch to origin (the branch with --force-with-lease).
push:
    #!/usr/bin/env bash
    set -euo pipefail
    branch=$(git branch --show-current)
    git push origin dev
    if [ "$branch" != dev ]; then
      git push --force-with-lease origin "$branch"
    fi
