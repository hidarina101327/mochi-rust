import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location("packager", Path(__file__).with_name("package-release.py"))
packager = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packager)


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def package(self, folder, name):
        directory = self.root / folder / name
        directory.mkdir(parents=True)
        metadata = {"id": name, "title": name, "version": "1.0.0", "summary": "独立包简介"}
        (directory / "marketplace.json").write_text(json.dumps(metadata), encoding="utf-8")
        if folder == "plugins":
            (directory / "manifest.json").write_text(json.dumps({"id": name, "version": "1.0.0"}), encoding="utf-8")
        elif folder == "workflows":
            (directory / "workflow.json").write_text("{}", encoding="utf-8")
        else:
            (directory / "模版.md").write_text(f"# {name}", encoding="utf-8")
        return directory

    def test_independent_zips_catalog_digests_and_reproducibility(self):
        for folder, names in [("plugins", ["github", "slack", "postgres"]), ("workflows", ["code-review", "research"]), ("templates", ["coding-agent"])]:
            for name in names:
                self.package(folder, name)
        one, two = self.root / "one", self.root / "two"
        items = packager.build(self.root, one, "market-v1")
        packager.build(self.root, two, "market-v1")
        expected = {"github-plugin.zip", "slack-plugin.zip", "postgres-plugin.zip", "code-review-workflow.zip", "research-workflow.zip", "coding-agent-template.zip"}
        self.assertEqual({p.name for p in one.glob("*.zip")}, expected)
        for item in items:
            data = (one / item["asset"]).read_bytes()
            self.assertEqual(hashlib.sha256(data).hexdigest(), item["sha256"])
            self.assertEqual(data, (two / item["asset"]).read_bytes())
            self.assertFalse(item["official"])
            with zipfile.ZipFile(one / item["asset"]) as archive:
                self.assertIn("marketplace.json", archive.namelist())
                self.assertFalse(any(n.startswith(("plugins/", "workflows/", "templates/")) for n in archive.namelist()))

    def test_local_cover_published_as_an_independent_image(self):
        directory = self.package("templates", "coding-agent")
        (directory / "cover.png").write_bytes(b"image fixture")
        path = directory / "marketplace.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        data.update(image="cover.png", official=True, category="agent")
        path.write_text(json.dumps(data), encoding="utf-8")
        items = packager.build(self.root, self.root / "out", "market-v1")
        self.assertTrue(items[0]["image"].endswith("/coding-agent-template-cover.png"))
        self.assertEqual((self.root / "out/coding-agent-template-cover.png").read_bytes(), b"image fixture")
        self.assertTrue(items[0]["official"])

    def test_missing_metadata_fails_without_partial_output(self):
        self.package("templates", "first")
        (self.root / "templates/second").mkdir()
        with self.assertRaises(FileNotFoundError):
            packager.build(self.root, self.root / "out", "market-v1")
        self.assertFalse((self.root / "out").exists())

    def test_refuses_empty_catalog_or_overwriting_release(self):
        with self.assertRaises(ValueError):
            packager.build(self.root, self.root / "out", "market-v1")
        self.package("templates", "coding-agent")
        packager.build(self.root, self.root / "out", "market-v1")
        with self.assertRaises(ValueError):
            packager.build(self.root, self.root / "out", "market-v1")

    def test_identity_rejected(self):
        directory = self.package("templates", "coding-agent")
        data = json.loads((directory / "marketplace.json").read_text(encoding="utf-8"))
        data["id"] = "../escape"
        (directory / "marketplace.json").write_text(json.dumps(data), encoding="utf-8")
        with self.assertRaises(ValueError):
            packager.build(self.root, self.root / "out", "market-v1")


if __name__ == "__main__":
    unittest.main()
