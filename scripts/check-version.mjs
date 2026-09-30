// Fails when the app's version is inconsistent across files (and, given a tag, when it differs from the tag).
//   node scripts/check-version.mjs [v1.2.3]
import fs from "node:fs";
const read = (p) => fs.readFileSync(new URL(`../${p}`, import.meta.url), "utf8");
const versions = {
  "apps/desktop/package.json": JSON.parse(read("apps/desktop/package.json")).version,
  "apps/desktop/src-tauri/tauri.conf.json": JSON.parse(read("apps/desktop/src-tauri/tauri.conf.json")).version,
  "apps/desktop/src-tauri/Cargo.toml": /^version\s*=\s*"([^"]+)"/m.exec(read("apps/desktop/src-tauri/Cargo.toml"))?.[1],
  "CHANGELOG.md (latest entry)": /^##\s+v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?)/m.exec(read("CHANGELOG.md"))?.[1],
};
const tag = process.argv[2]?.replace(/^refs\/tags\//, "");
if (tag) versions[`git tag ${tag}`] = tag.replace(/^v/, "");
const distinct = new Set(Object.values(versions));
for (const [file, v] of Object.entries(versions)) console.log(`${String(v).padEnd(12)} ${file}`);
if (distinct.size !== 1 || distinct.has(undefined)) {
  console.error("\nVersion mismatch: make every file above agree before releasing.");
  process.exit(1);
}
console.log(`\nAll versions agree: ${[...distinct][0]}`);
