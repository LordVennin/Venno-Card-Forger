/**
 * Read Proxyshop's layer-name constants straight out of `src/enums/layers.py`.
 *
 * We parse the Python source at survey time instead of porting the list, so the
 * survey always reflects the reference clone it is pointed at.
 */
export type KnownLayerNames = Map<string, string[]>; // layer name -> ["LAYERS.WHITE", ...]

export function parseLayerNameEnums(pythonSource: string): KnownLayerNames {
  const names: KnownLayerNames = new Map();
  let currentClass: string | undefined;
  for (const line of pythonSource.split(/\r?\n/)) {
    const cls = /^class\s+(\w+)/.exec(line);
    if (cls) {
      currentClass = cls[1];
      continue;
    }
    if (!currentClass) continue;
    const member = /^\s+([A-Z][A-Z0-9_]*)\s*=\s*(['"])(.*?)\2\s*(?:#.*)?$/.exec(line);
    if (!member) continue;
    const key = `${currentClass}.${member[1]}`;
    const value = member[3]!;
    const list = names.get(value);
    if (list) list.push(key);
    else names.set(value, [key]);
  }
  return names;
}
