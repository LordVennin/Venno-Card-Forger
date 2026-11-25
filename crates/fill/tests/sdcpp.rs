//! The stable-diffusion.cpp tier, with a stand-in for the `sd-cli` program:
//! a shell script that reads the fill script it is given and answers the way
//! the real one is documented to. What the real program does is not tested here.
#![cfg(all(feature = "sdcpp", unix))]

use cardsmith_fill::sdcpp::SdCpp;
use cardsmith_fill::{fill_with_fallback, FillProvider, FillRequest, PatchMatch, Placed};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cardsmith-sdcpp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Put an executable shell script in `dir` and return its path.
fn program(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("sd-cli");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Pulls a quoted value out of the fill script: `value_of input_path "$script"`.
const VALUE_OF: &str = r#"value_of() { sed -n "s/^ *$1 = \"\(.*\)\"$/\1/p" "$2" | head -1; }"#;

fn art() -> Vec<u8> {
    // Blue above, green below.
    (0..40u32).flat_map(|y| (0..40u32).map(move |_| y)).flat_map(|y| if y < 20 { [40, 80, 200, 255] } else { [50, 160, 60, 255] }).collect()
}

fn request(art: &[u8]) -> FillRequest<'_> {
    FillRequest { art: Placed { width: 40, height: 40, rgba: art, x: 30.0, y: 30.0, scale: 1.0 }, canvas: (100, 100), keep: [32.0, 32.0, 68.0, 68.0], seed: 9, prompt: Some("a \"quoted\" meadow") }
}

#[test]
fn it_says_what_is_missing_before_it_is_asked_to_run() {
    let dir = folder("missing");
    let mut tier = SdCpp::new(dir.join("sd-cli"), dir.join("model.ckpt"), dir.join("run"), 100);
    assert!(tier.available().reason.unwrap().contains("sd-cli program of stable-diffusion.cpp was not found"));
    program(&dir, "exit 0");
    let state = tier.available();
    assert!(state.reason.unwrap().contains("model has not been downloaded") && state.vram_mb == Some(4000));
    std::fs::write(dir.join("model.ckpt"), b"x").unwrap();
    assert!(tier.available().ok);
    // Not available means passed over, with the reason.
    std::fs::remove_file(dir.join("model.ckpt")).unwrap();
    let art = art();
    let outcome = fill_with_fallback(&mut [&mut tier, &mut PatchMatch { work_width: 100 }], &request(&art)).unwrap();
    assert_eq!(outcome.provider, "patchmatch");
    assert!(outcome.notices[0].starts_with("sdcpp is not available: its model has not been downloaded"));
}

#[test]
fn the_program_is_given_a_script_a_picture_and_a_mask_and_its_answer_guides_the_fill() {
    let dir = folder("run");
    std::fs::write(dir.join("model.ckpt"), b"x").unwrap();
    // The stand-in checks it was called as documented, then "paints" by handing the picture back.
    let body = format!(
        r#"{VALUE_OF}
[ "$1" = "-M" ] && [ "$2" = "dgl" ] && [ "$3" = "--script" ] || {{ echo "bad arguments: $*" >&2; exit 2; }}
[ "$5" = "--max-vram" ] && [ "$6" = "6" ] || {{ echo "extra arguments were not passed" >&2; exit 2; }}
script="$4"
[ -f "$(value_of mask_path "$script")" ] && [ -f "$(value_of weights "$script")" ] || {{ echo "a file named in the script is missing" >&2; exit 3; }}
cp "$(value_of input_path "$script")" "$(value_of output_path "$script")""#
    );
    let mut tier = SdCpp::new(program(&dir, &body), dir.join("model.ckpt"), dir.join("run"), 100);
    tier.extra_args = vec!["--max-vram".into(), "6".into()];
    let art = art();
    let outcome = fill_with_fallback(&mut [&mut tier, &mut PatchMatch { work_width: 100 }], &request(&art)).unwrap();
    assert_eq!((outcome.provider, outcome.notices.len()), ("sdcpp", 0), "{:?}", std::fs::read_to_string(dir.join("run/log.txt")));
    let filled = outcome.filled.unwrap();
    assert_eq!((filled.width, filled.height), (100, 100));
    // Where the art is trusted the fill is clear (it is not a second copy of the art); the rest is filled.
    let at = |x: u32, y: u32| { let i = ((y * 100 + x) * 4) as usize; filled.rgba[i + 3] };
    assert_eq!((at(50, 40), at(50, 60), at(2, 2), at(97, 97)), (0, 0, 255, 255));

    // What it was given: the seed, the prompt with its quotes escaped, absolute paths, and a mask that is white where the card is missing.
    let script = std::fs::read_to_string(dir.join("run/fill.dgl")).unwrap();
    assert!(script.contains("seed = 9") && script.contains(r#"prompt = "a \"quoted\" meadow""#) && script.contains("strength = 1.0"), "{script}");
    assert!(script.contains(&format!("input_path = \"{}", dir.join("run").canonicalize().unwrap().display())));
    let mask = std::fs::read(dir.join("run/mask.png")).unwrap();
    let picture = std::fs::read(dir.join("run/input.png")).unwrap();
    assert!(mask.starts_with(b"\x89PNG") && picture.starts_with(b"\x89PNG"));
}

#[test]
fn a_failure_in_the_program_is_reported_and_the_next_tier_fills_instead() {
    let dir = folder("fail");
    std::fs::write(dir.join("model.ckpt"), b"x").unwrap();
    let art = art();
    // It runs out of memory.
    let mut tier = SdCpp::new(program(&dir, "echo '[E] vulkan: out of device memory' >&2; exit 1"), dir.join("model.ckpt"), dir.join("run"), 100);
    let outcome = fill_with_fallback(&mut [&mut tier, &mut PatchMatch { work_width: 100 }], &request(&art)).unwrap();
    assert_eq!(outcome.provider, "patchmatch");
    assert!(outcome.notices[0].contains("sdcpp failed: sd-cli ended with") && outcome.notices[0].contains("out of device memory"), "{:?}", outcome.notices);
    assert!(outcome.filled.is_some());
    // It ends happily but writes nothing.
    let mut tier = SdCpp::new(program(&dir, "echo done"), dir.join("model.ckpt"), dir.join("run"), 100);
    let error = tier.fill(&request(&art)).err().unwrap();
    assert!(error.contains("sd-cli made no picture"), "{error}");
    // It hangs: stopped after the time allowed.
    let mut tier = SdCpp::new(program(&dir, "sleep 30"), dir.join("model.ckpt"), dir.join("run"), 100);
    tier.timeout = std::time::Duration::from_millis(300);
    let started = std::time::Instant::now();
    let error = tier.fill(&request(&art)).err().unwrap();
    assert!(error.contains("was stopped after") && started.elapsed().as_secs() < 10, "{error}");
}
