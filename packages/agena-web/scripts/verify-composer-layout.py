"""Verify first-line paint, scrolling and control containment in the real composer.

Start the Web Vite dev server, then run this script with Python + Playwright.
It launches isolated headless Chrome contexts with synthetic drafts.
"""
import argparse
import json
from pathlib import Path

from playwright.sync_api import expect, sync_playwright


def geometry(editor):
    return editor.evaluate("""el => {
      const shell = el.closest('.composer-shell');
      const status = [...shell.children].find(c => c.classList.contains('top-0'));
      const controls = shell.querySelector('[data-composer-controls]');
      const e = el.getBoundingClientRect(), s = shell.getBoundingClientRect();
      const style = getComputedStyle(el);
      const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
      const text = walker.nextNode();
      let first = null;
      if (text?.length) {
        const range = document.createRange();
        range.setStart(text, 0); range.setEnd(text, 1);
        const r = range.getBoundingClientRect();
        const hit = document.elementFromPoint(r.x + r.width / 2, r.y + 1);
        first = { top: r.y, bottom: r.bottom, paint_hits_editor: hit === el || el.contains(hit) };
      }
      return { editor_top: e.y, editor_bottom: e.bottom, editor_height: e.height,
        header_bottom: status?.getBoundingClientRect().bottom ?? null,
        shell_bottom: s.bottom, controls_bottom: controls.getBoundingClientRect().bottom,
        minimum_line_height: parseFloat(style.lineHeight) + parseFloat(style.paddingTop) + parseFloat(style.paddingBottom),
        scroll_top: el.scrollTop, first };
    }""")


def assert_layout(editor, first_visible=True):
    metrics = geometry(editor)
    assert metrics['header_bottom'] is None or metrics['editor_top'] >= metrics['header_bottom'] - 0.5, metrics
    assert metrics['editor_height'] >= metrics['minimum_line_height'], metrics
    assert metrics['controls_bottom'] <= metrics['shell_bottom'] + 0.5, metrics
    if first_visible and metrics['first']:
        first = metrics['first']
        assert first['top'] >= metrics['editor_top'] and first['bottom'] <= metrics['editor_bottom'], metrics
        assert first['paint_hits_editor'], metrics
    return metrics


def verify(browser, base, output):
    errors, results = [], {}
    for touch in [False, True]:
        tag = 'mobile' if touch else 'desktop'
        context = browser.new_context(viewport={'width': 390 if touch else 1280, 'height': 844 if touch else 900}, has_touch=touch)
        page = context.new_page()
        page.on('pageerror', lambda error: errors.append(str(error)))
        page.goto(base + '/tests/fixtures/composer-layout.html', wait_until='networkidle')
        editor = page.locator('[data-chat-input]')
        editor.fill('详细查看')
        page.wait_for_timeout(100)
        cases = {'single_line': assert_layout(editor)}
        if not touch:
            page.evaluate('composerLayoutFixture.height = 112')
            page.wait_for_timeout(100)
            cases['minimum_height'] = assert_layout(editor)
            page.evaluate('composerLayoutFixture.height = 128')
        editor.fill('\n'.join(f'第 {i} 行内容' for i in range(1, 51)))
        expect(editor).to_be_focused()
        page.wait_for_timeout(100)
        cases['long_draft'] = assert_layout(editor, first_visible=False)
        assert cases['long_draft']['scroll_top'] > 0, cases['long_draft']
        editor.evaluate('(el) => { el.scrollTop = 0 }')
        cases['scrolled_to_first_line'] = assert_layout(editor)
        editor.fill('详细查看')
        cases['short_after_long_draft'] = assert_layout(editor)
        page.evaluate('composerLayoutFixture.headerHeight = 36')
        page.wait_for_timeout(100)
        cases['larger_status_row'] = assert_layout(editor)
        page.evaluate('composerLayoutFixture.header = false')
        page.wait_for_timeout(100)
        cases['no_status_row'] = assert_layout(editor)
        assert cases['no_status_row']['editor_height'] > cases['larger_status_row']['editor_height']
        page.evaluate('composerLayoutFixture.header = true; composerLayoutFixture.headerHeight = 28; composerLayoutFixture.fullscreen = true')
        page.wait_for_timeout(100)
        cases['fullscreen'] = assert_layout(editor)
        page.evaluate('composerLayoutFixture.fullscreen = false')
        page.wait_for_timeout(100)
        cases['restored_compact_editor'] = assert_layout(editor)
        page.locator('[data-composer-pane]').screenshot(path=str(output / (tag + '.png')))
        results[tag] = cases
        context.close()
    assert not errors, errors
    return {'cases': results, 'page_errors': errors}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--base', default='http://127.0.0.1:5173')
    parser.add_argument('--output', default='/tmp/agena-composer-layout')
    args = parser.parse_args()
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        browser = p.chromium.launch(channel='chrome', headless=True)
        try:
            result = verify(browser, args.base, output)
            (output / 'verification.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
            print(json.dumps({'verified': sum(len(cases) for cases in result['cases'].values()), 'page_errors': result['page_errors']}))
        except Exception as error:
            print(f'Composer layout verification failed: {error}', flush=True)
            raise
        finally:
            browser.close()


if __name__ == '__main__':
    main()
