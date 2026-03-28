"""Anthropic SDK round-trip tests against llmposter.

Run with: uv run --with 'anthropic==0.*' python3 sdk-roundtrip-anthropic.py
Expects llmposter running on http://127.0.0.1:19871 with sdk-roundtrip.yaml fixtures.
"""

import sys

import anthropic

BASE = "http://127.0.0.1:19871"
client = anthropic.Anthropic(api_key="dummy", base_url=BASE)
PASS = 0
FAIL = 0


def check(name: str, fn):
    global PASS, FAIL
    try:
        fn()
        print(f"  PASS  {name}")
        PASS += 1
    except Exception as e:
        print(f"  FAIL  {name}: {e}")
        FAIL += 1


def test_non_streaming_text():
    r = client.messages.create(
        model="claude-sonnet-4-6",
        max_tokens=1024,
        messages=[{"role": "user", "content": "sdk-roundtrip-hello"}],
    )
    assert "llmposter" in r.content[0].text, repr(r.content[0].text)


def test_streaming_text():
    chunks = []
    with client.messages.stream(
        model="claude-sonnet-4-6",
        max_tokens=1024,
        messages=[{"role": "user", "content": "sdk-roundtrip-hello"}],
    ) as stream:
        chunks = list(stream.text_stream)
    assert "llmposter" in "".join(chunks), repr(chunks)


def test_tool_use():
    r = client.messages.create(
        model="claude-sonnet-4-6",
        max_tokens=1024,
        messages=[{"role": "user", "content": "sdk-roundtrip-tool"}],
        tools=[
            {
                "name": "get_weather",
                "description": "Get weather",
                "input_schema": {"type": "object", "properties": {}},
            }
        ],
    )
    tc = next(b for b in r.content if b.type == "tool_use")
    assert tc.name == "get_weather", repr(tc)
    assert tc.input["location"] == "San Francisco", repr(tc.input)


check("non-streaming text", test_non_streaming_text)
check("streaming text", test_streaming_text)
check("tool use", test_tool_use)

print(f"\nResults: {PASS} passed, {FAIL} failed")
if FAIL:
    sys.exit(1)
