"""Regression checks for production part clicks, live output and keyboard focus.

Start the Web Vite dev server, then run with Python + Playwright. This launches
an isolated headless Chrome context; no user browser profile is opened.
"""
import argparse
import json
from pathlib import Path

from playwright.sync_api import expect, sync_playwright


def verify(page, output, url):
    errors = []
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.goto(url, wait_until="networkidle")
    rows = page.locator("[data-part-id]:not([data-part-kind=lifecycle])")
    expect(rows).to_have_count(5)
    summary = page.locator('[data-part-kind="activity_summary"]')
    load = summary.locator('button[data-transcript-toggle="true"]')
    number = summary.locator('[data-part-page-size="true"]')
    all_parts = summary.locator('[data-part-collect-all="true"]')

    # Both the native focus path and the text-cursor path can reach the row.
    summary.focus()
    page.keyboard.press("ArrowRight")
    expect(load).to_be_focused()
    page.keyboard.press("ArrowRight")
    expect(number).to_be_focused()
    number.fill("3")
    page.keyboard.press("ArrowRight")
    expect(all_parts).to_be_focused()
    page.keyboard.press("ArrowLeft")
    expect(number).to_be_focused()
    page.keyboard.press("Enter")
    expect(rows).to_have_count(8)
    collapse = summary.locator('[data-part-collapse="true"]')
    expect(collapse).to_be_visible()
    all_parts.focus()
    page.keyboard.press("Enter")
    expect(rows).to_have_count(12)
    expect(collapse).to_be_focused()
    page.keyboard.press("Enter")
    expect(rows).to_have_count(5)
    expect(load).to_be_focused()

    shell = page.locator('[data-part-id="8"]')
    headline = shell.locator('[data-transcript-vim-toggle="true"]')
    expect(headline).to_have_attribute("aria-expanded", "false")
    bounds = headline.bounding_box()
    icon_bounds = headline.locator('[data-part-disclosure-icon]').bounding_box()
    assert icon_bounds["width"] >= 19 and icon_bounds["height"] >= 19
    assert abs(icon_bounds["y"] + icon_bounds["height"] / 2 - bounds["y"] - bounds["height"] / 2) < 1

    # A tiny pointer movement and a live revision between down/up used to
    # start transcript selection and swallow this click.
    page.evaluate('partControlsFixture.holdContent = true')
    x, y = bounds["x"] + 70, bounds["y"] + bounds["height"] / 2
    page.mouse.move(x, y)
    page.mouse.down()
    page.evaluate("partControlsFixture.revise()")
    page.mouse.move(x + 2, y + 1)
    page.mouse.up()
    expect(headline).to_have_attribute("aria-expanded", "true")
    output_widget = shell.locator('[data-content-output]')
    expect(output_widget).to_have_attribute('aria-busy', 'true')
    expect(output_widget.locator('[data-part-loading]')).to_be_visible()
    assert output_widget.locator('[data-part-loading] svg').evaluate("el => getComputedStyle(el).animationName") != 'none'
    page.screenshot(path=str(output / 'part-loading-output.png'))
    page.wait_for_function('partControlsFixture.contentRequests.length === 1')
    headline.locator('[data-part-disclosure-icon]').click()
    expect(headline).to_have_attribute('aria-expanded', 'false')
    expect(output_widget).to_have_count(0)
    page.wait_for_function('partControlsFixture.contentRequests[0].signal.aborted')
    page.evaluate('partControlsFixture.contentRequests[0].resolve(); partControlsFixture.holdContent = false')
    expect(headline).to_have_attribute('aria-expanded', 'false')
    headline.click()
    page.wait_for_function("partControlsFixture.streams.size > 0 && partControlsFixture.terminals.length > 0")
    expect(output_widget).to_have_attribute('aria-busy', 'false')
    expect(output_widget.locator('[data-part-loading]')).to_have_count(0)
    shell.locator('[data-tool-details-toggle]').click()
    page.evaluate('partControlsFixture.holdDetails = true')
    shell.locator('[data-tool-detail-section="output"]').click()
    detail_output = shell.locator('[data-tool-details] section:has([data-tool-detail-section="output"])')
    expect(detail_output.locator('[data-part-loading]')).to_be_visible()
    expect(detail_output.locator('[data-part-loading-placeholder]')).to_be_visible()
    page.screenshot(path=str(output / 'part-loading-details.png'))
    page.wait_for_function('partControlsFixture.detailRequests.length === 1')
    page.evaluate('partControlsFixture.holdDetails = false; partControlsFixture.detailRequests[0].resolve()')
    expect(detail_output).to_contain_text('tool detail revision 2')
    expect(detail_output.locator('[data-part-loading-placeholder]')).to_have_count(0)
    page.evaluate("partControlsFixture.holdDetails = true; partControlsFixture.revise()")
    page.wait_for_function("partControlsFixture.detailRequests.length === 2")
    expect(detail_output).to_contain_text('tool detail revision 2')
    page.evaluate("partControlsFixture.holdDetails = false; partControlsFixture.detailRequests[1].resolve()")
    expect(detail_output).to_contain_text('tool detail revision 3')
    page.evaluate("partControlsFixture.failNextDetails = true; partControlsFixture.revise()")
    expect(detail_output.locator('[role="alert"]')).to_contain_text('Temporary detail failure')
    expect(detail_output).to_contain_text('tool detail revision 3')
    page.evaluate("window.savedOutput = document.querySelector('[data-part-id=\"8\"] .xterm')")
    for sequence in range(20):
        page.evaluate("sequence => { partControlsFixture.emit(`live line ${sequence}\\n`); partControlsFixture.revise(); }", sequence)
    page.wait_for_function("partControlsFixture.terminals.at(-1).buffer.active.getLine(19)?.translateToString(true).includes('live line 19')")
    assert page.evaluate("savedOutput === document.querySelector('[data-part-id=\"8\"] .xterm')")
    page.evaluate("partControlsFixture.append(20)")
    expect(shell).to_be_attached()
    expect(headline).to_have_attribute("aria-expanded", "true")
    page.evaluate("partControlsFixture.finish()")
    expect(headline).to_have_attribute("aria-expanded", "true")
    assert page.evaluate("savedOutput === document.querySelector('[data-part-id=\"8\"] .xterm')")
    expect(shell.locator('[data-tool-details-toggle]')).to_have_attribute('aria-expanded', 'true')
    expect(detail_output).to_contain_text('tool detail revision 25', timeout=15000)
    expect(detail_output.locator('[role="alert"]')).to_have_count(0)
    expect(shell.get_by_text('Live output', exact=True)).to_have_count(0)

    # The same disclosure also has native keyboard collapse and reopening.
    headline.focus()
    page.keyboard.press("Space")
    expect(headline).to_have_attribute("aria-expanded", "false")
    page.keyboard.press("Enter")
    expect(headline).to_have_attribute("aria-expanded", "true")
    page.wait_for_function("partControlsFixture.terminals.at(-1).buffer.active.getLine(19)?.translateToString(true).includes('live line 19')")
    page.screenshot(path=str(output / "part-controls-expanded.png"), full_page=True)

    # Collapsing the list never hides an outstanding recovery prompt.
    collapse.click()
    expect(rows).to_have_count(5)
    page.evaluate("partControlsFixture.interaction()")
    interaction = page.locator('[data-part-id="3"]')
    expect(interaction).to_be_attached()
    expect(interaction.locator('[data-transcript-vim-toggle="true"]')).to_have_attribute("aria-expanded", "true")
    expect(interaction.get_by_text("Resume the operation?", exact=True)).to_be_visible()
    page.screenshot(path=str(output / "part-controls-pending.png"), full_page=True)
    # Default-open interactions must react to their first explicit collapse,
    # after another row already acquired a manual choice.
    interaction_headline = interaction.locator('[data-transcript-vim-toggle]')
    interaction_headline.click()
    expect(interaction_headline).to_have_attribute('aria-expanded', 'false')
    page.evaluate('partControlsFixture.message.parts[2].revision++')
    expect(interaction_headline).to_have_attribute('aria-expanded', 'false')
    interaction_headline.locator('[data-part-disclosure-icon]').click()
    expect(interaction_headline).to_have_attribute('aria-expanded', 'true')

    answer_id = page.evaluate('partControlsFixture.holdContent = true; partControlsFixture.text()')
    answer = page.locator(f'[data-part-id="{answer_id}"]')
    expect(answer.locator('[data-content-text]')).to_have_attribute('aria-busy', 'true')
    expect(answer.locator('[data-part-loading]')).to_be_visible()
    page.wait_for_function('partControlsFixture.contentRequests.some(r => r.resourceId === "loading-text")')
    answer_headline = answer.locator('[data-transcript-vim-toggle]')
    answer_headline.click()
    expect(answer_headline).to_have_attribute('aria-expanded', 'false')
    page.wait_for_function('partControlsFixture.contentRequests.at(-1).signal.aborted')
    page.evaluate('partControlsFixture.contentRequests.at(-1).resolve(); partControlsFixture.holdContent = false')
    expect(answer_headline).to_have_attribute('aria-expanded', 'false')
    answer_headline.click()
    expect(answer.locator('[data-content-text]')).to_have_attribute('aria-busy', 'false')
    expect(answer).to_contain_text('Loaded answer content')
    expect(answer.locator('[data-part-loading]')).to_have_count(0)
    assert not errors, errors
    return {"verified": ["left/right native and Vim-row navigation", "inline count and Enter",
        "load-all and keyboard collapse with focus recovery", "20px vertically centered disclosure",
        "click with pointer jitter during a live revision", "animated loading for initial output and tool details",
        "collapse during content loading aborts the read and ignores its late reply", "20 SSE chunks without replacing output DOM",
        "a loaded detail stays visible while the next revision is pending", "terminal detail catches up without closing its disclosure",
        "a temporary detail failure retains the last readable snapshot and recovers",
        "20 new siblings retain the open shell", "completion retains manual expansion and DOM",
        "Space collapse and Enter reopen with retained output", "pending recovery request defaults open and remains visible",
        "first mouse collapse of a default-open interaction", "default-open answer loads visibly, closes immediately, and reopens with content"],
        "page_errors": errors}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", default="http://127.0.0.1:5173/tests/fixtures/part-controls.html")
    parser.add_argument("--output", default="/tmp/agena-part-controls-visual")
    args = parser.parse_args()
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel="chrome", headless=True)
        page = browser.new_page(viewport={"width": 1280, "height": 1000})
        try:
            result = verify(page, output, args.url)
            (output / "part-controls-browser.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
            print(json.dumps(result, ensure_ascii=False))
        except Exception:
            page.screenshot(path=str(output / "part-controls-failure.png"), full_page=True)
            raise
        finally:
            browser.close()


if __name__ == "__main__":
    main()
