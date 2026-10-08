"""Measure production transcript spacing and verify adaptive output retention."""
import argparse
import json
from pathlib import Path

from playwright.sync_api import expect, sync_playwright


def verify(browser, base, output):
    errors = []
    page = browser.new_page(viewport={"width": 1280, "height": 900})
    page.on("pageerror", lambda error: errors.append(str(error)))
    page.goto(base + "/tests/fixtures/transcript-density.html", wait_until="networkidle")
    answer = page.locator('[data-part-kind="answer"]')
    expect(answer.locator('.oc-md-pre')).to_contain_text('1\n2\n3')
    metrics = page.evaluate("""() => {
      const height = selector => document.querySelector(selector).getBoundingClientRect().height;
      return { exchange_height: height('[data-density-example=exchange]'),
        markdown_height: height('[data-density-example=markdown]'),
        code_block_height: height('.oc-md-codeblock'),
        code_header_height: height('.oc-md-codeblock__header') };
    }""")
    assert metrics['exchange_height'] < 320, metrics
    assert metrics['markdown_height'] < 490, metrics
    expect(page.locator('[data-density-example="markdown"] pre')).to_contain_text('echo first\n\necho third')
    page.screenshot(path=str(output / 'compact-transcript.png'), full_page=True)
    page.close()

    mobile = browser.new_context(viewport={'width': 390, 'height': 844}, has_touch=True, is_mobile=True)
    page = mobile.new_page()
    page.on('pageerror', lambda error: errors.append(str(error)))
    page.goto(base + '/tests/fixtures/transcript-density.html', wait_until='networkidle')
    expect(page.locator('[data-part-kind="answer"] .oc-md-codeblock')).to_be_visible()
    assert page.evaluate('document.documentElement.scrollWidth <= innerWidth + 1')
    assert page.locator('[data-transcript-vim-toggle]').first.bounding_box()['height'] >= 36
    assert page.locator('.oc-md-codeblock__iconbtn').first.bounding_box()['height'] >= 36
    page.screenshot(path=str(output / 'compact-transcript-mobile.png'), full_page=True)
    # A full-page Chrome screenshot can reset touch emulation on its page.
    # A fresh page reapplies the context's device settings to the next check.
    page.close()
    page = mobile.new_page()
    page.on('pageerror', lambda error: errors.append(str(error)))
    page.goto(base + '/tests/fixtures/part-controls.html', wait_until='networkidle')
    assert page.evaluate('matchMedia("(pointer: coarse)").matches')
    page.evaluate('partControlsFixture.interaction()')
    interaction = page.locator('[data-transcript-interaction-part]').first
    expect(interaction).to_be_visible()
    for control in interaction.locator('label, button').all():
        if control.is_visible():
            assert control.bounding_box()['height'] >= 36, control.evaluate('(el) => ({tag: el.tagName, class: el.className, height: el.getBoundingClientRect().height, minHeight: getComputedStyle(el).minHeight, coarse: matchMedia("(pointer: coarse)").matches})')
    mobile.close()

    page = browser.new_page(viewport={'width': 1280, 'height': 1000})
    page.on('pageerror', lambda error: errors.append(str(error)))
    page.goto(base + '/tests/fixtures/part-controls.html', wait_until='networkidle')
    shell = page.locator('[data-part-id="8"]')
    shell.locator('[data-transcript-vim-toggle]').click()
    terminal = shell.locator('.content-terminal')
    page.wait_for_function('partControlsFixture.streams.size > 0')
    expect(shell.locator('[data-content-output]')).to_have_attribute('aria-busy', 'false')
    empty_height = terminal.bounding_box()['height']
    assert empty_height < 50, empty_height
    for text in ['1\n', '2\n', '3\n']:
        page.evaluate('text => partControlsFixture.emit(text)', text)
        page.wait_for_timeout(50)
    page.wait_for_function("partControlsFixture.terminals.at(-1).buffer.active.getLine(2)?.translateToString(true) === '3'")
    short_height = terminal.bounding_box()['height']
    assert short_height < 100, short_height
    page.evaluate('partControlsFixture.terminals.at(-1).select(0, 0, 1)')
    page.wait_for_function("partControlsFixture.terminals.at(-1).getSelection() === '1'")
    selected_height = terminal.bounding_box()['height']
    page.evaluate("() => { for (let i=4;i<=50;i++) partControlsFixture.emit(`${i}\\n`) }")
    page.wait_for_timeout(200)
    assert page.evaluate("partControlsFixture.terminals.at(-1).getSelection()") == '1'
    assert terminal.bounding_box()['height'] == selected_height, 'selection freezes display size along with output'
    page.evaluate('partControlsFixture.terminals.at(-1).clearSelection()')
    page.wait_for_function("partControlsFixture.terminals.at(-1).buffer.active.getLine(49)?.translateToString(true) === '50'")
    assert page.evaluate("partControlsFixture.terminals.at(-1).buffer.active.getLine(0).translateToString(true)") == '1'
    long_height = terminal.bounding_box()['height']
    assert long_height < 240, long_height
    page.screenshot(path=str(output / 'compact-live-output.png'))
    page.close()
    assert not errors, errors
    return {**metrics, 'empty_log_height':empty_height,'three_line_log_height':short_height,
      'long_log_height':long_height,'literal_code_blank_lines_preserved':True,
      'mobile_controls_at_least_36px':True,'selection_retains_text_and_size':True,
      'all_50_streamed_lines_retained':True,'page_errors':errors}


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--base',default='http://127.0.0.1:5173')
    parser.add_argument('--output',default='/tmp/agena-density-visual')
    args=parser.parse_args()
    output=Path(args.output);output.mkdir(parents=True,exist_ok=True)
    with sync_playwright() as p:
        browser=p.chromium.launch(channel='chrome',headless=True)
        try:
            result=verify(browser,args.base,output)
            (output/'density-browser.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
            print(json.dumps(result,ensure_ascii=False))
        finally:
            browser.close()


if __name__=='__main__':
    main()
