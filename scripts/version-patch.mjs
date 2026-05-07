import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "..");

const packageJsonPath = path.join(repoRoot, "package.json");
const packageLockPath = path.join(repoRoot, "package-lock.json");
const cargoTomlPath = path.join(repoRoot, "src-tauri", "Cargo.toml");

function incrementPatch(version) {
  const match = version.match(/^(\d+)\.(\d+)\.(\d+)(-.+)?$/);
  if (!match) {
    throw new Error(`Unsupported version format: ${version}`);
  }

  const [, major, minor, patch, suffix = ""] = match;
  return `${major}.${minor}.${Number(patch) + 1}${suffix}`;
}

function replaceCargoVersion(contents, nextVersion) {
  const packageSection = /(\[package\][\s\S]*?\nversion\s*=\s*")([^"]+)(")/;
  if (!packageSection.test(contents)) {
    throw new Error("Could not find package version in src-tauri/Cargo.toml");
  }

  return contents.replace(packageSection, `$1${nextVersion}$3`);
}

async function main() {
  const packageJson = JSON.parse(await readFile(packageJsonPath, "utf8"));
  const nextVersion = incrementPatch(packageJson.version);

  packageJson.version = nextVersion;
  await writeFile(packageJsonPath, `${JSON.stringify(packageJson, null, 2)}\n`);

  const packageLock = JSON.parse(await readFile(packageLockPath, "utf8"));
  packageLock.version = nextVersion;
  if (packageLock.packages?.[""]) {
    packageLock.packages[""] = {
      ...packageLock.packages[""],
      version: nextVersion,
    };
  }
  await writeFile(packageLockPath, `${JSON.stringify(packageLock, null, 2)}\n`);

  const cargoToml = await readFile(cargoTomlPath, "utf8");
  await writeFile(cargoTomlPath, replaceCargoVersion(cargoToml, nextVersion));

  process.stdout.write(`${nextVersion}\n`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});