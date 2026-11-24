# Trying the Stable Diffusion fill tier (Tier 3)

Tier 3 fills the space around card art with Stable Diffusion 1.5 inpainting, run by the
`sd-cli` program of [stable-diffusion.cpp](https://github.com/leejet/stable-diffusion.cpp).
It is the equivalent of Photoshop's Generative Fill, and it is opt-in: `--fill auto` never uses it.

**Status: written but not yet run against the real program.** It was built on a machine with no
graphics card. Its plumbing is tested with a stand-in program (`crates/fill/tests/sdcpp.rs`). The
first run on real hardware is the test. This page says how to do that run and what to report.

## The quick way

```
tools/install-tier3.sh                 # builds sd-cli for Vulkan and downloads the model
tools/install-tier3.sh --uninstall     # removes both, to start clean
```

It builds `sd-cli` from source for this machine (a few minutes), puts it and the model where
Cardsmith looks, and asks for `sudo` only to install missing build packages. After it, `--fill sd`
needs no `--sd-cli`. `--backend cuda` or `cpu` builds for those instead, `--no-model` skips the
download, `--ref <commit>` pins the stable-diffusion.cpp version. The CPU build has been tried;
the Vulkan and CUDA builds have not.

## What you need, by hand

1. **`sd-cli`** for your graphics card. Cardsmith does not download it.
   - The project's releases have a Linux Vulkan build
     (`sd-master-…-bin-Linux-Ubuntu-26.04-x86_64-vulkan.zip`). It is built on Ubuntu 26.04, so on an
     older system (Linux Mint 22 is Ubuntu 24.04 underneath) it may refuse to start with a message
     about `GLIBC`. If it does, build it from source, following the project's own build guide for
     the Vulkan (or CUDA) backend.
   - Keep the zip's files together: `sd-cli` needs the `.so` files beside it.
2. **The model**, 4.3 GB: `cardsmith models sd` downloads `sd-v1-5-inpainting.ckpt` into the cache
   (`~/.cache/cardsmith/scryfall/models/`). Its licence is CreativeML OpenRAIL-M. Any other SD 1.5
   *inpainting* checkpoint can be given with `--sd-model`.

## The run

```
cargo run --release -p cardsmith-cli -- build <template-dir> "<card name>" -o card.png \
    --fonts <fonts-dir> --art <art-image> \
    --fill sd --sd-cli /path/to/sd-cli
```

Options: `--sd-model <file>`, `--prompt "what should be there"` (the default asks for a seamless
continuation of the picture), `--seed <n>` for a variation, and `--sd-arg <arg>` (repeatable) to
pass arguments straight to `sd-cli`, for example `--sd-arg --max-vram --sd-arg 6` on a small card.
The `CARDSMITH_SD_CLI` environment variable can stand in for `--sd-cli`.

The last line printed says which tier made the fill: `filled around the art with sdcpp in … ms`.
If it says `lama` or `patchmatch`, Tier 3 was passed over, and the warning above it says why.

## If it does not work

Everything of the last run is kept in `~/.cache/cardsmith/scryfall/sdcpp/last-run/`:

| File | What it is |
|---|---|
| `fill.dgl` | The script `sd-cli` was given. |
| `input.png`, `mask.png` | The 512 × 512 picture and mask. White in the mask is what gets painted. |
| `output.png` | What `sd-cli` painted, if it got that far. |
| `log.txt` | Everything `sd-cli` printed. |

Two checks narrow a failure down:

```
/path/to/sd-cli -M dgl --script ~/.cache/cardsmith/scryfall/sdcpp/last-run/fill.dgl --check
/path/to/sd-cli -M dgl --script ~/.cache/cardsmith/scryfall/sdcpp/last-run/fill.dgl
```

The first checks the script without loading the model; the second runs it exactly as Cardsmith does.

## What to report

- Whether it ran, and the `filled around the art with …` line.
- `log.txt` if it failed, or if it fell back.
- How long it took and, if you can see it, how much graphics memory it used.
- Whether the fill looks better than Tier 2's (`--fill lama`) on the same card.

## What is known and not known

- The script is the project's own SD 1.5 inpainting example with Cardsmith's values filled in, and
  full strength (the missing area is painted from nothing). `sd-cli` has no command-line options
  for the picture, mask or prompt, which is why a script is written for every fill.
- That white means "paint here" is read from the example script, not stated in the project's
  documentation.
- The model works at 512 × 512. As with Tier 2, its answer is a guide: PatchMatch then fills at the
  working size from it, so the result has the art's own texture.
- Not known until it is run: memory use, speed, whether a `.ckpt` loads as readily as
  `.safetensors`, and what `sd-cli` prints and returns when it runs out of memory. A failure of any
  kind, or ten minutes without finishing, falls back to the next tier.
