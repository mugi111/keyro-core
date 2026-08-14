import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = fileURLToPath(new URL("..", import.meta.url));
const manifest = JSON.parse(readFileSync(join(packageRoot, "package.json"), "utf8"));

assert.equal(manifest.name, "@mugi111/keyro-protocol");
assert.equal(manifest.version, "0.3.0");
assert.equal(manifest.type, "module");
assert.equal(manifest.license, "Apache-2.0");
assert.equal(manifest.sideEffects, false);
assert.ok(!("." in manifest.exports), "package root must not expose an unversioned protocol");
assert.ok(!("./core-studio" in manifest.exports), "core-studio must be versioned");
assert.ok(!("./latest" in manifest.exports), "latest must not be exposed");
assert.ok(!("./package.json" in manifest.exports), "package.json must not be a public export");
assert.deepEqual(
  manifest.files.filter((path) => /v0\.(1|2)\.0/.test(path)),
  [],
  "package contents must ship only the current protocol version"
);

const currentTypes = manifest.exports["./core-studio/v0.3.0"]?.types;
assert.equal(currentTypes, "./generated/typescript/v0.3.0/core-studio.ts");
const currentRuntime = manifest.exports["./core-studio/v0.3.0"]?.import;
assert.equal(currentRuntime, "./generated/javascript/v0.3.0/core-studio.js");

const currentTypeSource = readFileSync(join(packageRoot, currentTypes), "utf8");
assert.match(currentTypeSource, /KEYRO_PROTOCOL_VERSION = "0\.3\.0"/);
assert.match(currentTypeSource, /KEYRO_PROTOCOL_HANDSHAKE_VERSION = \{/);
assert.match(currentTypeSource, /interface HandshakeProtocolVersion/);
assert.match(currentTypeSource, /major: 0;/);
assert.match(currentTypeSource, /minor: 3;/);
assert.match(currentTypeSource, /type: "create_profile"/);
assert.match(currentTypeSource, /type: "rename_profile"/);
assert.match(currentTypeSource, /type: "clear_assignment"/);
assert.match(currentTypeSource, /type: "profile"/);
assert.match(currentTypeSource, /type: "get_snapshot"/);
assert.match(currentTypeSource, /type: "snapshot"/);
assert.match(currentTypeSource, /actions: ActionDto\[\]/);

const currentRuntimeSource = readFileSync(join(packageRoot, currentRuntime), "utf8");
assert.match(currentRuntimeSource, /KEYRO_PROTOCOL_VERSION = "0\.3\.0"/);
assert.match(currentRuntimeSource, /KEYRO_PROTOCOL_HANDSHAKE_VERSION = \{/);
assert.ok(!currentRuntime.endsWith(".ts"), "runtime exports must not point at TypeScript source");

for (const [specifier, target] of Object.entries(manifest.exports)) {
  const paths = typeof target === "string" ? [target] : Object.values(target).filter((value) => typeof value === "string");
  for (const path of paths) {
    assert.ok(path.startsWith("./"), `${specifier} must export a package-relative path`);
    assert.ok(!path.includes(".."), `${specifier} must not escape the protocol package`);
    assert.ok(existsSync(join(packageRoot, path)), `${specifier} target does not exist: ${path}`);
  }
}

const schema = JSON.parse(readFileSync(join(packageRoot, "schemas/v0.3.0/core-studio.schema.json"), "utf8"));
const snapshotVector = JSON.parse(readFileSync(join(packageRoot, "test-vectors/v0.3.0/snapshot-response.json"), "utf8"));
const profileVector = JSON.parse(readFileSync(join(packageRoot, "test-vectors/v0.3.0/profile-response.json"), "utf8"));
assert.equal(schema.$id, "https://keyro.dev/protocol/v0.3.0/core-studio.schema.json");
assert.equal(schema.$defs.handshake.properties.protocol.$ref, "#/$defs/handshakeProtocolVersion");
assert.equal(schema.$defs.handshakeAccepted.properties.protocol.$ref, "#/$defs/handshakeProtocolVersion");
assert.equal(schema.$defs.handshakeProtocolVersion.properties.major.const, 0);
assert.equal(schema.$defs.handshakeProtocolVersion.properties.minor.const, 3);
assert.equal(snapshotVector.type, "snapshot");
assert.equal(profileVector.type, "profile");

console.log("@mugi111/keyro-protocol package metadata is aligned with v0.3.0 artifacts.");
