// Prints the CHANGELOG section for a version: node scripts/release-notes.mjs 1.0.0
import fs from "node:fs";
const version = (process.argv[2] ?? "").replace(/^v/, "");
const text = fs.readFileSync(new URL("../CHANGELOG.md", import.meta.url), "utf8");
const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const match = new RegExp(`^##\\s+v?${escaped}\\b[^\\n]*\\n([\\s\\S]*?)(?=^##\\s|$(?![\\s\\S]))`, "m").exec(text);
if (!match) {
  console.error(`No CHANGELOG entry for ${version}`);
  process.exit(1);
}
process.stdout.write(match[1].trim() + "\n");
