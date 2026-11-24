"""Makes tests/cards/art.png: a made-up scene (sky with clouds, hills, a
textured foreground, a few trees) that stands in for card art in the card
suite. It has enough structure for a fill around it to be judged by eye, and
it is our own, so it can be committed. Run: python3 tools/make-test-art.py
(needs numpy and Pillow). The picture is the same every time.
"""
import numpy as np
from PIL import Image

rng = np.random.default_rng(5)
w, h = 900, 660

def noise(scale, octaves=4):
    out, amp = np.zeros((h, w)), 1.0
    for o in range(octaves):
        s = max(2, scale // (2 ** o))
        small = rng.random((h // s + 2, w // s + 2))
        out += np.asarray(Image.fromarray((small * 255).astype("uint8")).resize((w, h), Image.BICUBIC)) / 255 * amp
        amp /= 2
    return out / out.max()

y, x = np.mgrid[0:h, 0:w]
t = y / h
img = np.stack([60 + 90 * t, 120 + 80 * t, 200 + 30 * t], axis=2)
img = img + (np.clip(noise(200) - 0.55, 0, 1) * 4)[..., None] * np.array([150, 140, 120])
for i, (base, amp, col) in enumerate([(0.52, 0.10, (70, 90, 120)), (0.62, 0.08, (50, 80, 70)), (0.74, 0.06, (60, 100, 50))]):
    ridge = (base + amp * (noise(230, 3)[int(h * 0.1 * (i + 1))] - 0.5) * 2) * h
    texture = noise(30, 3)[..., None] * 40 - 20
    img = np.where((y > ridge[None, :])[..., None], np.array(col)[None, None, :] + texture + (t[..., None] - base) * 60, img)
for _ in range(9):
    cx, gy, r = rng.integers(30, w - 30), int(h * (0.78 + 0.15 * rng.random())), rng.integers(14, 30)
    crown = (x - cx) ** 2 + ((y - (gy - 2 * r)) * 0.7) ** 2 < r * r
    img = np.where(crown[..., None], np.array([30, 70 + rng.integers(0, 30), 35])[None, None, :] + noise(9, 2)[..., None] * 30, img)
    img = np.where(((abs(x - cx) < 3) & (y > gy - 2 * r) & (y < gy))[..., None], np.array([60, 45, 30])[None, None, :], img)
Image.fromarray(np.clip(img, 0, 255).astype("uint8")).save("tests/cards/art.png", optimize=True)
