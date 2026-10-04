"""One action on an existing target. Requests use stdin; responses contain no page data."""
import asyncio
import json
import sys


async def act(request):
    try:
        from playwright.async_api import async_playwright, Error, TimeoutError, expect
    except ImportError:
        return {"ok": False, "error": "missing_dependency"}

    async with async_playwright() as playwright:
        # Contexts belong to Agena's retained native connection. Stopping this
        # driver disconnects it; do not close the browser, contexts or pages.
        browser = await playwright.chromium.connect_over_cdp(
            request["endpoint"], timeout=request["timeout_ms"]
        )
        page = None
        for context in browser.contexts:
            for candidate in context.pages:
                session = await context.new_cdp_session(candidate)
                try:
                    info = (await session.send("Target.getTargetInfo"))["targetInfo"]
                    if (info["targetId"] == request["target_id"]
                            and info.get("browserContextId") == request["context_id"]):
                        page = candidate
                finally:
                    await session.detach()
                if page is not None:
                    break
            if page is not None:
                break
        if page is None:
            return {"ok": False, "error": "target_missing"}
        page.set_default_timeout(request["timeout_ms"])
        scope = page.frame_locator("css=" + request["frame_selector"]) if request.get("frame_selector") else page
        target = None
        handle = None
        try:
            if request["action"] == "wait":
                if request.get("selector"):
                    await scope.locator("css=" + request["selector"]).wait_for(state="visible")
                elif request.get("text"):
                    await expect(scope.locator("body")).to_contain_text(
                        request["text"], use_inner_text=True, timeout=request["timeout_ms"]
                    )
                elif request.get("frame_selector"):
                    await scope.locator("body").wait_for(state="attached")
                else:
                    await page.wait_for_load_state("domcontentloaded")
            else:
                if request.get("selector"):
                    target = scope.locator("css=" + request["selector"])
                else:
                    try:
                        handle = await page.evaluate_handle(request["element_expression"])
                        target = handle.as_element()
                    except Error:
                        return {"ok": False, "error": "stale_reference"}
                    if target is None:
                        return {"ok": False, "error": "stale_reference"}
                if request["action"] == "click":
                    await target.click()
                elif request["action"] == "fill":
                    await target.fill(request["text"])
                    if request["press_enter"]:
                        await target.press("Enter")
                else:
                    return {"ok": False, "error": "action_failed"}
            return {"ok": True}
        except TimeoutError:
            return {"ok": False, "error": "timeout"}
        except Error as error:
            # Never serialize exception/call-log text: it can echo a password.
            code = "ambiguous" if "strict mode violation" in str(error) else "action_failed"
            return {"ok": False, "error": code}
        finally:
            if handle is not None:
                await handle.dispose()


async def main():
    try:
        request = json.loads(sys.stdin.buffer.read(128 * 1024 + 1))
        result = await asyncio.wait_for(act(request), request["timeout_ms"] / 1000)
    except asyncio.TimeoutError:
        result = {"ok": False, "error": "timeout"}
    except Exception:
        result = {"ok": False, "error": "action_failed"}
    print(json.dumps(result))


asyncio.run(main())
