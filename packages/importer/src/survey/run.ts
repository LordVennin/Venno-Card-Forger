import { readFile, readdir } from "node:fs/promises";
import { join, relative } from "node:path";
import { readPsd } from "ag-psd";
import { parseLayerNameEnums, type KnownLayerNames } from "./layerNames.js";
import { emptySurvey, finishSurvey, surveyPsd, type Survey } from "./survey.js";

/** Every .psd/.psb under `dir`, recursively, sorted. */
export async function findPsds(dir: string): Promise<string[]> {
  const entries = await readdir(dir, { recursive: true, withFileTypes: true });
  return entries
    .filter((e) => e.isFile() && /\.ps[db]$/i.test(e.name))
    .map((e) => join(e.parentPath, e.name))
    .sort();
}

/** Parse the structure of a PSD without decoding any pixels (no canvas needed). */
export function readPsdStructure(bytes: Uint8Array) {
  return readPsd(bytes, {
    skipLayerImageData: true,
    skipCompositeImageData: true,
    skipThumbnail: true,
  });
}

export async function surveyDirectory(
  dir: string,
  options: { layersPy?: string; onFile?: (file: string) => void } = {},
): Promise<Survey> {
  let names: KnownLayerNames | undefined;
  if (options.layersPy) names = parseLayerNameEnums(await readFile(options.layersPy, "utf8"));

  const survey = emptySurvey();
  for (const path of await findPsds(dir)) {
    const file = relative(dir, path);
    options.onFile?.(file);
    try {
      surveyPsd(readPsdStructure(await readFile(path)), file, survey, names);
    } catch (err) {
      survey.failed.push({ file, error: err instanceof Error ? err.message : String(err) });
    }
  }
  return finishSurvey(survey, names);
}
