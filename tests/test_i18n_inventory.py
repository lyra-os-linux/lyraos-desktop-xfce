from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/check-i18n-inventory.py"
SPEC = importlib.util.spec_from_file_location("check_i18n_inventory", SCRIPT)
assert SPEC and SPEC.loader
module = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = module
SPEC.loader.exec_module(module)


class I18nInventoryTests(unittest.TestCase):
    def test_inventory_contract_is_complete(self) -> None:
        self.assertEqual(module.validate(), [])

    def test_alpha4_packages_are_explicit(self) -> None:
        document = json.loads(module.INVENTORY.read_text(encoding="utf-8"))
        alpha4 = {entry["id"]: entry for entry in document["packages"] if entry["wave"] == "alpha4"}
        self.assertEqual(
            set(alpha4),
            {
                "lyra-icons",
                "lyra-installer",
                "lyra-theme",
                "lyra-wallpapers",
                "vega-gtk",
                "fina",
                "sheliak",
            },
        )
        self.assertEqual(alpha4["fina"]["status"], "complete")

    def test_noto_fonts_remain_image_policy(self) -> None:
        image = (ROOT / "kiwi/config.xml").read_text(encoding="utf-8")
        self.assertIn('<package name="google-noto-coloremoji-fonts"/>', image)
        self.assertIn('<package name="google-noto-sans-cjk-fonts"/>', image)


if __name__ == "__main__":
    unittest.main()
