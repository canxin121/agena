"""Verify canonical media artifacts with production Vue components.

Start Vite, then run with Python + Playwright. No provider calls are made.
"""
import argparse
import json
import os
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from playwright.sync_api import sync_playwright


def verify(page, url, output):
    requests = []

    def local_image(route):
        requests.append(parse_qs(urlparse(route.request.url).query))
        route.fulfill(status=200, content_type="image/svg+xml", body=(
            '<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360">'
            '<rect width="640" height="360" fill="#5eead4"/></svg>'
        ))

    page.route("**/api/v1/workbench/fs/raw?**", local_image)
    page.route("https://example.test/**", lambda route: route.fulfill(status=204))
    page.goto(url, wait_until="networkidle")
    page.wait_for_function("window.mediaFixture && document.querySelector('[data-fixture=inline] img')?.naturalWidth === 640")
    inline = page.locator('[data-fixture="inline"]')
    assert "generated-dashboard.svg" in inline.inner_text()
    assert "base64," not in inline.inner_text()
    inline.locator("img").click()
    page.wait_for_function("mediaFixture.ui.isImageViewerOpen")
    assert page.evaluate("mediaFixture.ui.imageViewerActiveItem.src === mediaFixture.inline.source.agenaPresentation.blocks[0].artifact.uri")
    page.keyboard.press("Escape")
    page.wait_for_function("!mediaFixture.ui.isImageViewerOpen")

    page.wait_for_function("document.querySelector('[data-fixture=local] img')?.naturalWidth === 640")
    assert requests, "local images must use the file API"
    assert requests[0]["directory"] == ["/workspace/session-781"]
    assert requests[0]["path"] == ["/workspace/session-781/UI draft.svg"]
    assert "file://" not in page.locator('[data-fixture="local"] img').get_attribute("src")

    assert page.locator('[data-fixture="video"] video[controls]').count() == 1
    assert page.locator('[data-fixture="audio"] audio[controls]').count() == 1
    assert page.locator('[data-fixture="running"] img').count() == 0
    page.evaluate("mediaFixture.complete()")
    page.wait_for_function("document.querySelector('[data-fixture=running] img')?.naturalWidth === 640")
    running = page.locator('[data-fixture="running"]')
    running.locator('[data-transcript-vim-toggle]').first.click()
    assert not page.evaluate("mediaFixture.expanded.running")
    running.locator('[data-transcript-vim-toggle]').first.click()
    page.wait_for_function("document.querySelector('[data-fixture=running] img')?.naturalWidth === 640")

    for width, name in [(1200, "desktop"), (390, "mobile")]:
        page.set_viewport_size({"width": width, "height": 1000})
        page.wait_for_function("document.documentElement.scrollWidth <= innerWidth")
        page.screenshot(path=str(output / f"operation-media-{name}.png"), full_page=True)
    result = {"verified": ["inline generated image decodes", "image viewer opens and closes", "local image uses owning session file API", "canonical audio and video controls", "running to completed image", "collapse and reopen", "desktop and mobile layout"]}
    (output / "operation-media.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default="http://127.0.0.1:5176/tests/fixtures/operation-media.html")
    parser.add_argument("--output", default="/tmp/agena-operation-media")
    args = parser.parse_args()
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as playwright:
        executable = os.environ.get("AGENA_WEBKIT_EXECUTABLE")
        browser = playwright.webkit.launch(headless=True, **({"executable_path": executable} if executable else {}))
        try:
            page = browser.new_page(viewport={"width": 1200, "height": 1000})
            page.set_default_timeout(15000)
            verify(page, args.url, output)
        finally:
            browser.close()


if __name__ == "__main__":
    main()
