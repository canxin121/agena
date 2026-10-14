"""Production workspace/part lifecycle regression and browser timings.

Start `bun scripts/part-content-server.mjs`, then run with Python + Playwright.
Uses an isolated headless Chrome profile and deterministic local session data.
Timings describe this fixture, not network latency or end-to-end production FPS.
"""
import argparse
import json
from pathlib import Path
from urllib.parse import urlsplit
from playwright.sync_api import sync_playwright


def measure(page, action):
    return page.evaluate("""async action => {
      const f = uiPerformanceFixture;
      const before = performance.now();
      if (action.kind === 'tab') f.ui.selectWorkspaceWindow(action.id);
      else if (action.kind === 'part') document.querySelector(
        `[data-workspace-pane-window="${f.ui.activeWorkspaceWindowId}"] [data-part-id="${f.lastPartId}"] [data-transcript-vim-toggle]`
      ).click();
      else [...document.querySelectorAll('nav > button')].find(b => b.textContent === action.label).click();
      const handler = performance.now() - before;
      await new Promise(resolve => requestAnimationFrame(() => setTimeout(resolve, 0)));
      return { handler_ms: handler, next_frame_ms: performance.now() - before };
    }""", action)


def verify(page, url, output):
    errors = []
    page.on('pageerror', lambda error: errors.append(str(error)))
    page.goto(url, wait_until='networkidle')
    page.wait_for_function('uiPerformanceFixture.mounts === 1')
    page.wait_for_function('document.querySelectorAll("[data-part-id]").length >= uiPerformanceFixture.pairs * 6')
    page.wait_for_timeout(250)
    page.evaluate("""() => {
      const f = uiPerformanceFixture;
      f.firstRoot = document.querySelector('[data-fixture-transcript]');
      f.firstRow = f.firstRoot.querySelector('[data-part-id]');
      f.firstRoot.scrollTop = 350;
    }""")
    first = page.evaluate('uiPerformanceFixture.firstId')
    page.evaluate("""() => {
      const f = uiPerformanceFixture;
      f.longTasks = [];
      f.longTaskObserver = new PerformanceObserver(list => f.longTasks.push(
        ...list.getEntries().map(entry => ({start_ms: entry.startTime, duration_ms: entry.duration}))
      ));
      f.longTaskObserver.observe({type: 'longtask'});
    }""")
    metrics = {'open': measure(page, {'label': 'Open tab'})}
    page.wait_for_function('uiPerformanceFixture.mounts === 2')
    second = page.evaluate('uiPerformanceFixture.ui.activeWorkspaceWindowId')
    page.wait_for_function("""() => document.querySelectorAll(
      `[data-workspace-pane-window="${uiPerformanceFixture.ui.activeWorkspaceWindowId}"] [data-part-id]`
    ).length === uiPerformanceFixture.pairs * 6""")
    page.wait_for_timeout(300)
    metrics['cold_open_long_tasks'] = page.evaluate("""() => {
      uiPerformanceFixture.longTaskObserver.disconnect();
      return uiPerformanceFixture.longTasks;
    }""")
    assert page.evaluate('uiPerformanceFixture.firstRow.isConnected'), 'hidden tab destroyed its transcript'
    page.evaluate("""() => {
      const f = uiPerformanceFixture;
      f.secondRoot = document.querySelector(`[data-workspace-pane-window="${f.ui.activeWorkspaceWindowId}"]`);
      f.secondRow = f.secondRoot.querySelector('[data-part-id]');
    }""")
    metrics['move_to_split'] = measure(page, {'label': 'Move to split'})
    page.wait_for_function('uiPerformanceFixture.ui.workspaceGroups.length === 2')
    assert page.evaluate('uiPerformanceFixture.mounts === 2 && uiPerformanceFixture.secondRow.isConnected')
    metrics['merge_splits'] = measure(page, {'label': 'Merge splits'})
    page.wait_for_function('uiPerformanceFixture.ui.workspaceGroups.length === 1')
    assert page.evaluate('uiPerformanceFixture.mounts === 2 && uiPerformanceFixture.unmounts === 0')
    assert page.evaluate('uiPerformanceFixture.secondRow.isConnected'), 'moving a pane replaced its content'

    metrics['warm_switches'] = [measure(page, {'kind': 'tab', 'id': first if i % 2 == 0 else second}) for i in range(12)]
    assert page.evaluate('uiPerformanceFixture.mounts === 2 && uiPerformanceFixture.unmounts === 0')
    measure(page, {'kind': 'tab', 'id': first})
    assert page.evaluate('uiPerformanceFixture.firstRow.isConnected && uiPerformanceFixture.firstRoot.scrollTop === 350')
    measure(page, {'kind': 'tab', 'id': second})

    page.locator(f'[data-workspace-pane-window="{second}"] [data-part-id]').last.scroll_into_view_if_needed()
    metrics['part_toggles'] = []
    for _ in range(6):
        metrics['part_toggles'].append(measure(page, {'kind': 'part'}))
        page.wait_for_timeout(100)

    # Offscreen message menus must not be instantiated en masse, but remain
    # fully usable when their message enters the viewport.
    lazy_message_id = page.evaluate("""() => {
      const root = uiPerformanceFixture.secondRoot;
      const row = [...root.querySelectorAll('article[data-role="user"]')].find(row =>
        !row.querySelector('[data-transcript-chrome="true"] button'));
      if (!row) throw new Error('all offscreen action menus were mounted eagerly');
      row.scrollIntoView({block: 'center'});
      return row.dataset.messageId;
    }""")
    lazy_message = page.locator(f'[data-workspace-pane-window="{second}"] article[data-message-id="{lazy_message_id}"]')
    lazy_message.get_by_role('button', name='Fork from here', exact=True).click()
    page.get_by_text('Fork session?', exact=True).wait_for(state='visible')
    page.get_by_role('button', name='Cancel', exact=True).click()

    metrics['duplicate_split'] = measure(page, {'label': 'Split'})
    page.wait_for_function('uiPerformanceFixture.mounts === 3')
    metrics['close_split'] = measure(page, {'label': 'Close tab'})
    page.wait_for_function('uiPerformanceFixture.unmounts === 1')
    page.wait_for_function('document.querySelectorAll("[data-workspace-pane-window]").length === 2')
    page.evaluate('uiPerformanceFixture.ui.selectWorkspaceWindow(uiPerformanceFixture.firstId)')
    # Opening and closing before the deferred page mount must not resurrect it.
    await_mounts = page.evaluate('uiPerformanceFixture.mounts')
    page.evaluate("""async () => {
      const ui = uiPerformanceFixture.ui;
      const id = ui.openWorkspaceWindow('chat', {query: {sessionId: 'rapid'}, reuseExisting: false});
      await Promise.resolve();
      ui.closeWorkspaceWindow(id);
    }""")
    page.wait_for_timeout(200)
    assert page.evaluate('uiPerformanceFixture.mounts') == await_mounts

    # Diff expansion must expose every line through scrolling without mounting
    # or highlighting thousands of rows in the click's render pass.
    page.evaluate("""() => uiPerformanceFixture.setDiff(
      '--- /dev/null\\n+++ b/large.ts\\n@@ -0,0 +1,12000 @@\\n' +
      Array.from({length: 12000}, (_, i) => '+const item' + i + ' = "<safe>";').join('\\n')
    )""")
    diff = page.locator('[data-transcript-diff]')
    diff.get_by_role('button', name='Show 11921 more lines', exact=True).click()
    viewport = diff.locator('[data-diff-viewport]')
    viewport.wait_for(state='visible')
    assert viewport.locator('[data-diff-line]').count() <= 40
    viewport.evaluate('el => { el.scrollTop = el.scrollHeight }')
    page.wait_for_function('document.querySelector("[data-diff-viewport]").textContent.includes("item11999")')
    assert viewport.locator('[data-diff-line]').count() <= 40
    diff.locator('button[aria-expanded="true"]').click()
    assert diff.locator('[data-diff-line]').count() == 80
    metrics['clipped_cursor'] = page.evaluate("""async () => {
      const {transcriptTextProjection} = uiPerformanceFixture;
      const root = document.createElement('div');
      root.style.cssText = 'position:fixed;left:10px;top:10px;width:240px;height:30px;overflow:hidden;white-space:pre;font:16px/30px monospace';
      root.textContent = 'abcd😀e\\u0301 '.repeat(1500);
      document.body.append(root);
      const graphemes = [...new Intl.Segmenter(undefined, {granularity:'grapheme'}).segment(root.textContent)];
      const native = Range.prototype.getClientRects;
      const results = [];
      try {
        for (const direction of ['ltr', 'rtl']) {
          root.dir = direction;
          const clip = root.getBoundingClientRect();
          const candidates = direction === 'ltr' ? graphemes.slice(0, 200) : graphemes.slice(-200);
          const expected = candidates.filter(g => {
            const range = document.createRange();
            range.setStart(root.firstChild, g.index);
            range.setEnd(root.firstChild, g.index + g.segment.length);
            return [...range.getClientRects()].some(r => r.width > 0 && r.height > 0 &&
              (r.left+r.right)/2 >= clip.left && (r.left+r.right)/2 <= clip.right);
          }).map(g => g.segment).join('');
          let reads = 0;
          Range.prototype.getClientRects = function(...args) { reads++; return native.apply(this, args) };
          const start = performance.now();
          const text = transcriptTextProjection(root).text;
          const ms = performance.now() - start;
          Range.prototype.getClientRects = native;
          if (text !== expected || !text) throw new Error('clipped Unicode text changed: '+direction);
          if (reads >= 100) throw new Error('cursor measured '+reads+' ranges for one clipped row');
          results.push({direction, range_reads: reads, ms});
        }
      } finally { Range.prototype.getClientRects = native; root.remove() }
      return results;
    }""")
    page.screenshot(path=str(output / 'workspace-and-large-diff.png'))
    assert not errors, errors
    return {'source': 'production Vue components / deterministic session data / isolated Chrome',
            'messages_per_tab': page.evaluate('uiPerformanceFixture.pairs * 2'),
            'metrics': metrics, 'page_errors': errors,
            'verified': ['retained hidden transcript and scroll offset', 'split move and merge preserve DOM and instances',
                         '12 warm switches without remount', 'part expand/collapse', 'duplicate split and cleanup',
                         'offscreen message menus mount on approach and their confirmation controls work',
                         'rapid close cancels pending mount', '12000-line diff has bounded DOM and reachable last line',
                         'clipped Unicode cursor agrees with grapheme geometry in LTR and RTL with fewer than 100 range reads']}


def verify_hidden_streams(page, url):
    origin = urlsplit(url)
    page.goto(f'{origin.scheme}://{origin.netloc}/tests/fixtures/part-content.html', wait_until='networkidle')
    page.wait_for_function('partFixture.streams.size === 3 && partFixture.terminals.size === 3')
    page.evaluate("""() => {
      const f = partFixture;
      f.savedTerminal = f.terminals.get('shell');
      f.savedElement = document.querySelector('[data-fixture=shell] .xterm');
      f.emit('shell', {type:'log', stream:'stdout', text:'before hide\\n'});
    }""")
    page.wait_for_function("partFixture.savedTerminal.buffer.active.getLine(0)?.translateToString(true) === 'before hide'")
    page.evaluate('partFixture.savedTerminal.selectAll()')
    page.evaluate('partFixture.setVisible(false)')
    page.wait_for_function('partFixture.streams.size === 0')
    page.evaluate("""() => {
      partFixture.emit('shell', {type:'log', stream:'stdout', text:'while hidden\\n'});
      partFixture.savedTerminal.clearSelection();
      partFixture.setVisible(true);
    }""")
    page.wait_for_function('partFixture.streams.size === 3')
    page.wait_for_function("partFixture.savedTerminal.buffer.active.getLine(1)?.translateToString(true) === 'while hidden'")
    assert page.evaluate("""partFixture.savedTerminal === partFixture.terminals.get('shell') &&
      partFixture.savedElement === document.querySelector('[data-fixture=shell] .xterm')""")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--url', default='http://127.0.0.1:5175/tests/fixtures/ui-interaction-performance.html')
    parser.add_argument('--output', default='/tmp/agena-ui-interaction-performance')
    args = parser.parse_args()
    output = Path(args.output)
    output.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        browser = p.chromium.launch(channel='chrome', headless=True)
        page = browser.new_page(viewport={'width': 1440, 'height': 1000})
        try:
            result = verify(page, args.url, output)
            verify_hidden_streams(page, args.url)
            assert not result['page_errors'], result['page_errors']
            result['verified'].append('hidden panes disconnect content streams, resume output and retain the terminal instance')
            (output / 'results.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
            print(json.dumps(result, ensure_ascii=False))
        except Exception:
            page.screenshot(path=str(output / 'failure.png'), full_page=True)
            raise
        finally:
            browser.close()


if __name__ == '__main__':
    main()
