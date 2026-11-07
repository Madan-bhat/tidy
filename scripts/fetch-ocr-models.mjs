#!/usr/bin/env node
/**
 * Download the ocrs ONNX models into src-tauri/resources/ocr/ so they ship
 * inside the app bundle. Users never install tesseract.
 */
import { createWriteStream, existsSync, mkdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { pipeline } from "node:stream/promises";
import { Readable } from "node:stream";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "src-tauri", "resources", "ocr");
mkdirSync(outDir, { recursive: true });

const MODELS = [
  {
    name: "text-detection.onnx",
    url: "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx",
    // Rough lower bound so we don't treat a failed HTML error page as the model.
    minBytes: 1_000_000,
  },
  {
    name: "text-recognition.onnx",
    url: "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx",
    minBytes: 1_000_000,
  },
];

async function fetchOne({ name, url, minBytes }) {
  const dest = join(outDir, name);
  if (existsSync(dest) && statSync(dest).size >= minBytes) {
    console.log(`OCR model present: ${name}`);
    return;
  }
  console.log(`Downloading OCR model ${name}…`);
  const res = await fetch(url);
  if (!res.ok || !res.body) {
    throw new Error(`Failed to download ${name}: HTTP ${res.status}`);
  }
  await pipeline(Readable.fromWeb(res.body), createWriteStream(dest));
  const size = statSync(dest).size;
  if (size < minBytes) {
    throw new Error(`${name} looks truncated (${size} bytes)`);
  }
  console.log(`Wrote ${dest} (${(size / 1e6).toFixed(1)} MB)`);
}

for (const model of MODELS) {
  await fetchOne(model);
}
console.log("OCR models ready.");
