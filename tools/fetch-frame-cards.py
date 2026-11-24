"""Fetches the Scryfall data for every card named in
crates/cards/tests/data/frame_data.toml and writes the fields the card logic
reads to crates/cards/tests/data/frame_cards.json, so the frame logic tests
run without the network.

Run from the repository root after `cargo build --release -p cardsmith-cli`:
    python3 tools/fetch-frame-cards.py

It goes through `cardsmith card`, so requests are spaced and cached as
Scryfall asks. Only game data is kept: no images, prices, flavor text or ids.
"""
import json, re, subprocess, sys, tomllib

CARD = ["object", "name", "layout", "mana_cost", "type_line", "oracle_text", "colors", "color_identity", "color_indicator", "power", "toughness", "loyalty", "defense",
        "keywords", "frame_effects", "frame", "set", "set_name", "set_type", "collector_number", "rarity", "lang", "released_at", "promo", "promo_types", "oracle_id"]
PART = ["object", "component", "name", "type_line", "uri"]

def trim(card):
    out = {k: card[k] for k in CARD if k in card}
    if "card_faces" in card:
        out["card_faces"] = [trim(face) for face in card["card_faces"]]
    if "all_parts" in card:
        out["all_parts"] = [{k: part[k] for k in PART if k in part} for part in card["all_parts"]]
    return out

def fetch(*args):
    done = subprocess.run(["target/release/cardsmith", "card", "--raw", *args], capture_output=True, text=True)
    if done.returncode:
        sys.exit(f"{args}: {done.stderr.strip()}")
    return json.loads(done.stdout)

cases = tomllib.load(open("crates/cards/tests/data/frame_data.toml", "rb"))
cards = {}
for group in cases.values():
    for key in group:
        if key in cards:
            continue
        found = re.search(r"\[(.*)]", key)
        name = key.replace(found.group(0), "").strip() if found else key
        card = fetch(name, *(["--set", found.group(1)] if found else []))
        cards[key] = trim(card)
        # A meld card needs the faces of its pair: the other half and what they become.
        if card.get("layout") == "meld":
            parts = [p for p in card.get("all_parts", []) if p.get("component") in ("meld_part", "meld_result")]
            cards[key]["meld_parts"] = {p["name"]: trim(fetch(p["name"], "--set", card["set"])) for p in parts}
        print(key, "->", card["set"], card["layout"], file=sys.stderr)
json.dump({"//": "Scryfall game data for the cards in frame_data.toml, trimmed by tools/fetch-frame-cards.py. Card data courtesy of Scryfall.", "cards": cards},
          open("crates/cards/tests/data/frame_cards.json", "w"), indent=1, ensure_ascii=False, sort_keys=True)
print(len(cards), "cards")
