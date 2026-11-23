/**
 * A minimal PNG writer for Node. The importer writes hundreds of layer images,
 * some of them card-sized, so this uses zlib directly at a fast setting.
 */
import { crc32, deflateSync } from "node:zlib";

function chunk(type: string, data: Uint8Array): Buffer {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, "latin1");
  out.set(data, 8);
  out.writeUInt32BE(crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
  return out;
}

/**
 * Encode 8-bit pixels as PNG. `channels` is 4 for straight-alpha RGBA or 1 for
 * grayscale; `pixels` is row-major with no padding.
 */
export function encodePng(width: number, height: number, pixels: Uint8Array, channels: 1 | 4 = 4): Buffer {
  const stride = width * channels;
  if (pixels.length !== stride * height) throw new Error(`PNG: expected ${stride * height} bytes, got ${pixels.length}`);
  // Each scanline is prefixed with its filter type; 0 (none) keeps this simple and fast.
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw.set(pixels.subarray(y * stride, (y + 1) * stride), y * (stride + 1) + 1);
  }
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8; // bits per channel
  header[9] = channels === 4 ? 6 : 0; // color type: RGBA or grayscale
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw, { level: 3 })),
    chunk("IEND", new Uint8Array(0)),
  ]);
}
