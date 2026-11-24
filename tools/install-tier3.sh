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

args=("$@")
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

# Started from a file manager with no terminal: there is nowhere to show progress or ask for
# a password, so open a terminal and run there.
if [ ! -t 1 ] && [ -z "${CARDSMITH_INSTALL_NO_TERMINAL:-}" ] && [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]; then
    for term in x-terminal-emulator gnome-terminal xfce4-terminal konsole xterm; do
        if command -v "$term" >/dev/null; then
            case "$term" in
                gnome-terminal) exec "$term" -- "$0" "${args[@]}" ;;
                *) exec "$term" -e "$0" "${args[@]}" ;;
            esac
        fi
    done
fi

# Everything shown is also kept in a log, so a failure can be read (and sent on) afterwards.
mkdir -p "$cache"
log="$cache/install-tier3.log"
exec > >(tee "$log") 2>&1
echo "install-tier3.sh ${args[*]} on $(date)"

# A terminal opened just for this script closes when the script ends, taking the error with
# it. Wait for Enter in that case. (SHLVL is 1 there, 2 or more when run from a shell.)
finish() {
    code=$?
    if [ $code -ne 0 ]; then
        echo
        echo "The install FAILED (exit code $code). The log is at $log"
    fi
    if [ -t 0 ] && [ "${SHLVL:-1}" -le 1 ]; then
        read -r -p "Press Enter to close this window. " _ || true
    fi
    sleep 0.2 # let the log catch up
}
trap finish EXIT
program="$cache/sdcpp/sd-cli"
source_dir="$cache/sdcpp/src"
model_file="$cache/models/sd-v1-5-inpainting.ckpt"

if [ "$uninstall" = 1 ]; then
    rm -rf "$cache/sdcpp" "$model_file" "$model_file.part" "$cache/install-tier3.log"
    echo "Removed $cache/sdcpp and $model_file"
    exit 0
fi

case "$backend" in
    vulkan) flags=(-DSD_VULKAN=ON) ;;
    cuda)   flags=(-DSD_CUDA=ON) ;;
    cpu)    flags=() ;;
    *) echo "--backend must be vulkan, cuda or cpu" >&2; exit 2 ;;
esac

# What the build needs, as "command:package". Asked for by command, so a tool installed
# some other way (the Vulkan SDK's glslc, say) counts.
needs=(git:git cmake:cmake g++:build-essential make:build-essential)
[ "$backend" = vulkan ] && needs+=(glslc:glslc)

# --- 1. sd-cli ---------------------------------------------------------------
if [ -x "$program" ] && [ "$force" = 0 ]; then
    echo "sd-cli is already at $program (use --force to rebuild)"
else
    missing=()
    for need in "${needs[@]}"; do
        command -v "${need%%:*}" >/dev/null || missing+=("${need##*:}")
    done
    if [ "$backend" = vulkan ] && [ ! -e /usr/include/vulkan/vulkan.h ] && [ -z "${VULKAN_SDK:-}" ]; then
        missing+=(libvulkan-dev)
    fi
    if [ ${#missing[@]} -gt 0 ]; then
        mapfile -t missing < <(printf '%s\n' "${missing[@]}" | sort -u)
        if [ "$deps" = 0 ] || ! command -v apt-get >/dev/null; then
            echo "These packages are needed and missing: ${missing[*]}. Install them and run this again." >&2
            exit 1
        fi
        echo "==> Installing build packages: ${missing[*]} (sudo will ask for your password)"
        if ! sudo apt-get install -y "${missing[@]}"; then
            echo >&2
            echo "apt could not install them. If the one it could not find is glslc (older Ubuntu and Mint" >&2
            echo "have no such package), install the Vulkan SDK from https://vulkan.lunarg.com, or use" >&2
            echo "--backend cpu to try the rest without the graphics card." >&2
            exit 1
        fi
    fi
    if [ "$backend" = cuda ] && ! command -v nvcc >/dev/null; then
        echo "The cuda backend needs the CUDA toolkit (nvcc was not found). Install it, or use --backend vulkan." >&2
        exit 1
    fi

    echo "==> Downloading the stable-diffusion.cpp source ($ref)"
    rm -rf "$source_dir"
    mkdir -p "$source_dir"
    git -C "$source_dir" init -q
    git -C "$source_dir" remote add origin https://github.com/leejet/stable-diffusion.cpp
    git -C "$source_dir" fetch --depth 1 origin "$ref"
    git -C "$source_dir" checkout -q FETCH_HEAD
    git -C "$source_dir" submodule update --init --recursive --depth 1

    # A compiler can take 2 GB on the biggest files; more of them at once than memory allows
    # freezes the machine.
    jobs="$(nproc)"
    mem_gb="$(awk '/MemAvailable/ {print int($2 / 1048576)}' /proc/meminfo 2>/dev/null || echo 0)"
    if [ "${mem_gb:-0}" -gt 0 ] && [ "$jobs" -gt $((mem_gb / 2)) ]; then
        jobs=$((mem_gb / 2))
    fi
    [ "$jobs" -lt 1 ] && jobs=1
    echo "==> Building stable-diffusion.cpp $(git -C "$source_dir" rev-parse --short HEAD) for $backend, $jobs at a time; this takes several minutes"

    cmake -S "$source_dir" -B "$source_dir/build" -DCMAKE_BUILD_TYPE=Release "${flags[@]}"
    nice cmake --build "$source_dir/build" --config Release -j "$jobs"

    built="$(find "$source_dir/build" -type f -name sd-cli -perm -u+x | head -n 1)"
    if [ -z "$built" ]; then
        echo "The build finished but made no sd-cli." >&2
        exit 1
    fi
    cp "$built" "$program"
    # Shared libraries, if this version builds any, go beside the program.
    find "$source_dir/build" -type f -name '*.so*' -exec cp {} "$cache/sdcpp/" \;
    git -C "$source_dir" rev-parse HEAD >"$cache/sdcpp/version.txt"
    rm -rf "$source_dir"
    echo "==> sd-cli ($backend) is at $program"
fi

# --- 2. the model ------------------------------------------------------------
if [ "$model" = 1 ]; then
    if [ -f "$model_file" ] && [ "$force" = 0 ]; then
        echo "The model is already at $model_file"
    else
        # Straight from Hugging Face, with a progress bar; an interrupted download carries on
        # where it stopped. (The same file `cardsmith models sd` downloads.)
        url=https://huggingface.co/stable-diffusion-v1-5/stable-diffusion-inpainting/resolve/main/sd-v1-5-inpainting.ckpt
        want=4265437280
        mkdir -p "$(dirname "$model_file")"
        echo "==> Downloading the model, 4.3 GB, to $model_file"
        if command -v curl >/dev/null; then
            curl -L --fail --retry 3 -C - --progress-bar -o "$model_file.part" "$url"
        elif command -v wget >/dev/null; then
            wget -c --show-progress -q -O "$model_file.part" "$url"
        else
            echo "Neither curl nor wget is installed; install one, or run: cargo run --release -p cardsmith-cli -- models sd" >&2
            exit 1
        fi
        have="$(stat -c %s "$model_file.part")"
        if [ "$have" != "$want" ]; then
            echo "The model came out $have bytes long where $want were expected. Run this again to carry on." >&2
            exit 1
        fi
        mv "$model_file.part" "$model_file"
    fi
fi

echo
echo "Ready. Try it with:"
echo "  cargo run --release -p cardsmith-cli -- build <template-dir> \"<card name>\" -o card.png --fonts <fonts-dir> --art <art-image> --fill sd"
