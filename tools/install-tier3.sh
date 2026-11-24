#!/usr/bin/env bash
# Install what the Stable Diffusion fill tier (--fill sd) needs, where Cardsmith looks for it:
#
#   1. sd-cli, built from the stable-diffusion.cpp source for this machine
#      -> <cache>/sdcpp/sd-cli
#   2. the SD 1.5 inpainting model, 4.3 GB
#      -> <cache>/models/sd-v1-5-inpainting.ckpt
#
# After it, `cardsmith build ... --fill sd` works with no --sd-cli or --sd-model.
# It is safe to run again: what is already there is kept unless --force is given.
#
#   tools/install-tier3.sh [--backend vulkan|cuda|cpu] [--ref <branch, tag or commit>]
#                          [--cache <dir>] [--no-model] [--no-deps] [--force]
#   tools/install-tier3.sh --uninstall [--cache <dir>]     # remove both, to start clean
#
# See docs/TIER3.md for the test run itself.
set -euo pipefail

backend=vulkan
ref=master
cache="${XDG_CACHE_HOME:-$HOME/.cache}/cardsmith/scryfall"
model=1 deps=1 force=0 uninstall=0

while [ $# -gt 0 ]; do
    case "$1" in
        --backend) backend="$2"; shift 2 ;;
        --ref) ref="$2"; shift 2 ;;
        --cache) cache="$2"; shift 2 ;;
        --no-model) model=0; shift ;;
        --no-deps) deps=0; shift ;;
        --force) force=1; shift ;;
        --uninstall) uninstall=1; shift ;;
        -h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
    esac
done

repo="$(cd "$(dirname "$0")/.." && pwd)"
program="$cache/sdcpp/sd-cli"
source_dir="$cache/sdcpp/src"
model_file="$cache/models/sd-v1-5-inpainting.ckpt"

if [ "$uninstall" = 1 ]; then
    rm -rf "$cache/sdcpp" "$model_file" "$model_file.part"
    echo "Removed $cache/sdcpp and $model_file"
    exit 0
fi

case "$backend" in
    vulkan) flags=(-DSD_VULKAN=ON); packages=(git cmake build-essential libvulkan-dev glslc) ;;
    cuda)   flags=(-DSD_CUDA=ON);   packages=(git cmake build-essential) ;;
    cpu)    flags=();               packages=(git cmake build-essential) ;;
    *) echo "--backend must be vulkan, cuda or cpu" >&2; exit 2 ;;
esac

# --- 1. sd-cli ---------------------------------------------------------------
if [ -x "$program" ] && [ "$force" = 0 ]; then
    echo "sd-cli is already at $program (use --force to rebuild)"
else
    if [ "$deps" = 1 ] && command -v apt-get >/dev/null; then
        missing=()
        for p in "${packages[@]}"; do
            dpkg -s "$p" >/dev/null 2>&1 || missing+=("$p")
        done
        if [ ${#missing[@]} -gt 0 ]; then
            echo "Installing build packages: ${missing[*]}"
            sudo apt-get install -y "${missing[@]}"
        fi
    fi
    if [ "$backend" = cuda ] && ! command -v nvcc >/dev/null; then
        echo "The cuda backend needs the CUDA toolkit (nvcc was not found). Install it, or use --backend vulkan." >&2
        exit 1
    fi

    rm -rf "$source_dir"
    mkdir -p "$source_dir"
    git -C "$source_dir" init -q
    git -C "$source_dir" remote add origin https://github.com/leejet/stable-diffusion.cpp
    git -C "$source_dir" fetch -q --depth 1 origin "$ref"
    git -C "$source_dir" checkout -q FETCH_HEAD
    git -C "$source_dir" submodule update -q --init --recursive --depth 1
    echo "Building stable-diffusion.cpp $(git -C "$source_dir" rev-parse --short HEAD) for $backend; this takes a few minutes"

    cmake -S "$source_dir" -B "$source_dir/build" -DCMAKE_BUILD_TYPE=Release "${flags[@]}" >"$cache/sdcpp/build.log" 2>&1 \
        && cmake --build "$source_dir/build" --config Release -j "$(nproc)" >>"$cache/sdcpp/build.log" 2>&1 \
        || { echo "The build failed. Its last lines ($cache/sdcpp/build.log):" >&2; tail -n 25 "$cache/sdcpp/build.log" >&2; exit 1; }

    built="$(find "$source_dir/build" -type f -name sd-cli -perm -u+x | head -n 1)"
    if [ -z "$built" ]; then
        echo "The build finished but made no sd-cli; see $cache/sdcpp/build.log" >&2
        exit 1
    fi
    cp "$built" "$program"
    # Shared libraries, if this version builds any, go beside the program.
    find "$source_dir/build" -type f -name '*.so*' -exec cp {} "$cache/sdcpp/" \;
    git -C "$source_dir" rev-parse HEAD >"$cache/sdcpp/version.txt"
    rm -rf "$source_dir"
    echo "sd-cli ($backend) is at $program"
fi

# --- 2. the model ------------------------------------------------------------
if [ "$model" = 1 ]; then
    if [ -f "$model_file" ] && [ "$force" = 0 ]; then
        echo "The model is already at $model_file"
    else
        (cd "$repo" && cargo run --release -q -p cardsmith-cli -- models sd --cache "$cache")
    fi
fi

echo
echo "Ready. Try it with:"
echo "  cargo run --release -p cardsmith-cli -- build <template-dir> \"<card name>\" -o card.png --fonts <fonts-dir> --art <art-image> --fill sd"
