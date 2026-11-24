import { afterEach, beforeEach, describe, expect, it } from "vitest";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { syncDir } from "../../scripts/sync_dist.mjs";

// `app/dist` feeds `tauri::generate_context!`, which makes cargo recompile the
// `ui` crate when any embedded file has a newer mtime. The sync must therefore
// leave byte-identical files untouched.
describe("syncDir", () => {
  let root: string;
  let src: string;
  let dest: string;

  const write = (base: string, rel: string, body: string) => {
    const file = path.join(base, rel);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, body);
  };
  const old = new Date("2020-01-01T00:00:00Z");

  beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "sync-dist-"));
    src = path.join(root, "staging");
    dest = path.join(root, "dist");
  });
  afterEach(() => fs.rmSync(root, { recursive: true, force: true }));

  it("keeps the mtime of unchanged files and rewrites only changed ones", () => {
    write(src, "index.html", "<html>");
    write(src, "assets/a.js", "same");
    write(src, "assets/b.js", "new body");
    write(dest, "index.html", "<html>");
    write(dest, "assets/a.js", "same");
    write(dest, "assets/b.js", "old body");
    for (const rel of ["index.html", "assets/a.js", "assets/b.js"]) {
      fs.utimesSync(path.join(dest, rel), old, old);
    }

    const result = syncDir(src, dest);

    expect(result).toEqual({ written: ["assets/b.js"], removed: [] });
    expect(fs.statSync(path.join(dest, "index.html")).mtime).toEqual(old);
    expect(fs.statSync(path.join(dest, "assets/a.js")).mtime).toEqual(old);
    expect(fs.readFileSync(path.join(dest, "assets/b.js"), "utf8")).toBe("new body");
    expect(fs.statSync(path.join(dest, "assets/b.js")).mtime.getTime()).toBeGreaterThan(old.getTime());
  });

  it("adds new files, removes stale files and prunes emptied directories", () => {
    write(src, "assets/new.js", "new");
    write(dest, "assets/stale.js", "stale");
    write(dest, "gone/deep/old.css", "old");

    const result = syncDir(src, dest);

    expect(result.written).toEqual(["assets/new.js"]);
    expect(result.removed.sort()).toEqual(["assets/stale.js", "gone/deep/old.css"]);
    expect(fs.existsSync(path.join(dest, "gone"))).toBe(false);
    expect(fs.readdirSync(path.join(dest, "assets"))).toEqual(["new.js"]);
  });

  it("creates the destination when it does not exist", () => {
    write(src, "index.html", "x");
    expect(syncDir(src, dest).written).toEqual(["index.html"]);
    expect(fs.readFileSync(path.join(dest, "index.html"), "utf8")).toBe("x");
  });

  it("replaces a directory that became a file", () => {
    write(src, "x", "file");
    write(dest, "x/inner.js", "dir");
    syncDir(src, dest);
    expect(fs.readFileSync(path.join(dest, "x"), "utf8")).toBe("file");
  });
});
