import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

/** Regenerate cargo-dist CI with the project's draft-only release policy. */
function main() {
  const root = fileURLToPath(new URL("../", import.meta.url));
  const configPath = new URL("../dist-workspace.toml", import.meta.url);
  const workflowPath = new URL("../.github/workflows/release.yml", import.meta.url);
  const config = readFileSync(configPath, "utf8");
  const original = readFileSync(workflowPath, "utf8");
  const check = process.argv.includes("--check");
  let completed = false;

  try {
    // Temporarily enable cargo-dist's generator; never leave the source config changed.
    const enabled = config.replace(/^allow-dirty = \["ci"\]\r?\n/m, "");
    assert.notEqual(enabled, config, "Expected the documented CI override in dist-workspace.toml");
    writeFileSync(configPath, enabled);
    const result = spawnSync(process.env.PROP_DIST_BIN || "dist", ["generate", "--mode", "ci"], {
      cwd: root,
      stdio: "inherit",
    });
    if (result.error) throw result.error;
    assert.equal(result.status, 0, "cargo-dist generation failed");
    let generated = readFileSync(workflowPath, "utf8");

    const commands = generated.match(/^          gh release create .+$/gm) || [];
    assert.equal(commands.length, 1, "Review the updated cargo-dist release template before adapting it");
    assert(!generated.includes("gh release edit"), "Unexpected upstream release edit command");
    const create = commands[0].trim()
      .replace('"${{ needs.plan.outputs.tag }}"', '"$RELEASE_TAG"')
      .replace("gh release create ", "gh release create --draft --verify-tag ");
    assert(create.includes('"$RELEASE_TAG"'), "Expected the release tag in cargo-dist's create command");
    const draftOnly = [
      '          if gh release view "$RELEASE_TAG" --json isDraft --jq .isDraft > "$RUNNER_TEMP/is-draft.txt" 2>/dev/null; then',
      '            if [ "$(cat "$RUNNER_TEMP/is-draft.txt")" != "true" ]; then',
      '              echo "Refusing to modify an already published release: $RELEASE_TAG" >&2',
      '              exit 1',
      '            fi',
      '            gh release edit "$RELEASE_TAG" --draft=true --title "$ANNOUNCEMENT_TITLE" --notes-file "$RUNNER_TEMP/notes.txt"',
      '            gh release upload "$RELEASE_TAG" artifacts/* --clobber',
      "          else",
      `            ${create}`,
      "          fi",
    ].join("\n");
    generated = generated.replace(commands[0], draftOnly);
    generated = generated.replace(
      '          RELEASE_COMMIT: "${{ github.sha }}"',
      '          RELEASE_TAG: "${{ needs.plan.outputs.tag }}"\n          RELEASE_COMMIT: "${{ github.sha }}"',
    );
    generated = generated.replace(
      "      - name: Cache dist\n",
      "      - name: Verify draft release workflow\n        run: node scripts/generate-release.mjs --check\n      - name: Cache dist\n",
    );
    assert(generated.includes("Verify draft release workflow"), "Expected cargo-dist's plan job");
    generated = "# Regenerate with: node scripts/generate-release.mjs\n# Releases remain drafts until explicitly published by a maintainer.\n" + generated;
    if (check) {
      assert.equal(original, generated, "Regenerate the release workflow with node scripts/generate-release.mjs");
    } else {
      writeFileSync(workflowPath, generated);
    }
    completed = true;
    console.log(check ? "Draft release workflow is up to date." : "Draft release workflow regenerated.");
  } finally {
    writeFileSync(configPath, config);
    if (check || !completed) writeFileSync(workflowPath, original);
  }
}

main();
