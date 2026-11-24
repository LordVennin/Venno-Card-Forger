"""Fetches the cards Proxyshop renders to test its normal layout (the [normal]
section of src/data/tests/template_renders.toml) and writes them, as the card
engine's own card data, to tests/cards/cards.json for `pnpm test:cards`.

Run from the repository root after `cargo build --release -p cardsmith-cli`:
    python3 tools/fetch-suite-cards.py

Flavor text is not kept: where a card has some, a line written for this suite
stands in, so the flavor divider and spacing are still exercised.
"""
import json, subprocess, sys, tomllib

STAND_INS = [
    "It had waited a long time to be found.",
    "\"The mountain keeps what it is given.\"\n—Miners' saying",
    "The first settlers named the valley for its silence. The second wave, arriving after the bells were hung, could not imagine why.",
]

cases = tomllib.load(open("reference/Proxyshop/src/data/tests/template_renders.toml", "rb"))["normal"]
cards = []
for name, what in cases.items():
    done = subprocess.run(["target/release/cardsmith", "card", name], capture_output=True, text=True)
    if done.returncode:
        sys.exit(f"{name}: {done.stderr.strip()}")
    card = json.loads(done.stdout)
    if card["flavorText"]:
        card["flavorText"] = STAND_INS[len(cards) % len(STAND_INS)]
    cards.append({"case": what, "card": card})
    print(name, "->", card["set"], card["frame"], file=sys.stderr)
json.dump({"//": "The cards of Proxyshop's normal-layout render test, as card data, made by tools/fetch-suite-cards.py. Flavor text is replaced by lines written for this suite. Card data courtesy of Scryfall.", "cards": cards},
          open("tests/cards/cards.json", "w"), indent=1, ensure_ascii=False)
print(len(cards), "cards")
