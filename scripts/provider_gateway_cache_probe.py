#!/usr/bin/env python3

import argparse
import json
import ssl
import sys
import time
import urllib.error
import urllib.parse
import urllib.request


def post_json(url: str, payload: dict, headers: dict[str, str]) -> dict:
    data = json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(url, data=data, headers=headers, method="POST")
    context = ssl.create_default_context()
    with urllib.request.urlopen(request, context=context, timeout=180) as response:
        body = response.read().decode("utf-8")
        return json.loads(body)


def cache_hit(value: object) -> bool:
    return isinstance(value, int) and value > 0


def probe_openai(base_url: str, api_key: str, model: str, nonce: str) -> dict:
    text = (f"CACHE-MARKER-OPENAI-{nonce} ") * 4000
    payload = {
        "model": model,
        "input": text,
        "max_output_tokens": 32,
        "prompt_cache_key": f"agena-openai-cache-test-{nonce}",
    }
    headers = {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {api_key}",
    }
    url = f"{base_url}/api/provider/openai/v1/responses"
    first = post_json(url, payload, headers)
    second = post_json(url, payload, headers)
    return {
        "first_input_tokens": first.get("usage", {}).get("input_tokens"),
        "first_cached_tokens": first.get("usage", {})
        .get("input_tokens_details", {})
        .get("cached_tokens"),
        "second_input_tokens": second.get("usage", {}).get("input_tokens"),
        "second_cached_tokens": second.get("usage", {})
        .get("input_tokens_details", {})
        .get("cached_tokens"),
    }


def probe_claude(base_url: str, api_key: str, model: str, nonce: str) -> dict:
    system = (f"CACHE-MARKER-CLAUDE-SYSTEM-{nonce} ") * 4000
    payload = {
        "model": model,
        "max_tokens": 64,
        "metadata": {"user_id": f"agena-cache-user-claude-{nonce}"},
        "system": system,
        "tools": [
            {
                "name": "project_search",
                "description": "Search project files for matches.",
                "input_schema": {
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "required": ["query"],
                },
            }
        ],
        "messages": [{"role": "user", "content": "Reply with OK only."}],
    }
    headers = {
        "content-type": "application/json",
        "x-api-key": api_key,
        "anthropic-version": "2023-06-01",
    }
    url = f"{base_url}/api/provider/claude/v1/messages"
    first = post_json(url, payload, headers)
    second = post_json(url, payload, headers)
    return {
        "first_input_tokens": first.get("usage", {}).get("input_tokens"),
        "first_cache_read_input_tokens": first.get("usage", {}).get(
            "cache_read_input_tokens"
        ),
        "second_input_tokens": second.get("usage", {}).get("input_tokens"),
        "second_cache_read_input_tokens": second.get("usage", {}).get(
            "cache_read_input_tokens"
        ),
    }


def probe_gemini(base_url: str, api_key: str, model: str, nonce: str) -> dict:
    text = (f"CACHE-MARKER-GEMINI-{nonce} ") * 4000
    payload = {
        "contents": [{"role": "user", "parts": [{"text": text}]}],
        "generationConfig": {"maxOutputTokens": 32},
    }
    headers = {"content-type": "application/json"}
    encoded_key = urllib.parse.quote(api_key, safe="")
    url = (
        f"{base_url}/api/provider/gemini/v1beta/models/"
        f"{urllib.parse.quote(model, safe='')}:generateContent?key={encoded_key}"
    )
    first = post_json(url, payload, headers)
    second = post_json(url, payload, headers)
    first_usage = first.get("usageMetadata", {})
    second_usage = second.get("usageMetadata", {})
    return {
        "first_prompt_tokens": first_usage.get("promptTokenCount"),
        "first_cached_content_tokens": first_usage.get("cachedContentTokenCount"),
        "second_prompt_tokens": second_usage.get("promptTokenCount"),
        "second_cached_content_tokens": second_usage.get("cachedContentTokenCount"),
    }


def probe_with_retries(
    label: str,
    attempts: int,
    retry_delay_secs: float,
    probe_fn,
    hit_key: str,
) -> dict:
    observations: list[dict] = []
    best_hit_value = 0

    for attempt in range(1, attempts + 1):
        observation = dict(probe_fn())
        observation["attempt"] = attempt
        observations.append(observation)

        hit_value = observation.get(hit_key)
        if cache_hit(hit_value):
            best_hit_value = max(best_hit_value, int(hit_value))
            break

        if isinstance(hit_value, int):
            best_hit_value = max(best_hit_value, hit_value)

        if attempt < attempts and retry_delay_secs > 0:
            time.sleep(retry_delay_secs)

    final = dict(observations[-1])
    final["attempt_count"] = len(observations)
    final["cache_hit_observed"] = any(cache_hit(item.get(hit_key)) for item in observations)
    final["max_hit_value"] = best_hit_value
    final["hit_key"] = hit_key
    final["attempts"] = observations
    final["provider_label"] = label
    return final


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Probe authoritative cache behavior exposed by provider-routed gateway "
            "OpenAI, Claude, and Gemini endpoints."
        )
    )
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--api-key", required=True)
    parser.add_argument("--model", default="gpt-5.4")
    parser.add_argument(
        "--attempts",
        type=int,
        default=1,
        help="Maximum number of probe attempts to run for each provider.",
    )
    parser.add_argument(
        "--retry-delay-secs",
        type=float,
        default=0.0,
        help="Seconds to sleep between retry attempts that did not observe a cache hit.",
    )
    parser.add_argument(
        "--nonce",
        default=str(int(time.time() * 1000)),
        help="Unique suffix used to force a cold first request.",
    )
    args = parser.parse_args()
    attempts = max(args.attempts, 1)
    retry_delay_secs = max(args.retry_delay_secs, 0.0)

    base_url = args.base_url.rstrip("/")
    result = {
        "base_url": base_url,
        "model": args.model,
        "nonce": args.nonce,
        "attempts_configured": attempts,
        "retry_delay_secs": retry_delay_secs,
        "openai": probe_with_retries(
            "openai",
            attempts,
            retry_delay_secs,
            lambda: probe_openai(base_url, args.api_key, args.model, args.nonce),
            "second_cached_tokens",
        ),
        "claude": probe_with_retries(
            "claude",
            attempts,
            retry_delay_secs,
            lambda: probe_claude(base_url, args.api_key, args.model, args.nonce),
            "second_cache_read_input_tokens",
        ),
        "gemini": probe_with_retries(
            "gemini",
            attempts,
            retry_delay_secs,
            lambda: probe_gemini(base_url, args.api_key, args.model, args.nonce),
            "second_cached_content_tokens",
        ),
    }
    json.dump(result, sys.stdout, ensure_ascii=True, indent=2)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        print(
            json.dumps(
                {
                    "status": error.code,
                    "reason": error.reason,
                    "body": body,
                },
                ensure_ascii=True,
                indent=2,
            ),
            file=sys.stderr,
        )
        raise SystemExit(1)
