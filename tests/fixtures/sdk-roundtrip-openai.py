"""OpenAI SDK round-trip tests against llmposter.

Run with: uv run --with 'openai==1.*' python3 sdk-roundtrip-openai.py
Expects llmposter running on http://127.0.0.1:19871 with sdk-roundtrip.yaml fixtures.
"""

import json
import sys

import openai

BASE = "http://127.0.0.1:19871/v1"
client = openai.OpenAI(api_key="dummy", base_url=BASE)
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
    r = client.chat.completions.create(
        model="gpt-4",
        messages=[{"role": "user", "content": "sdk-roundtrip-hello"}],
    )
    assert r.choices[0].message.content == "Hello from llmposter mock!", repr(r)


def test_streaming_text():
    chunks = [
        c.choices[0].delta.content
        for c in client.chat.completions.create(
            model="gpt-4",
            messages=[{"role": "user", "content": "sdk-roundtrip-hello"}],
            stream=True,
        )
        if c.choices and c.choices[0].delta.content
    ]
    assert "llmposter" in "".join(chunks), repr(chunks)


def test_tool_call():
    r = client.chat.completions.create(
        model="gpt-4",
        messages=[{"role": "user", "content": "sdk-roundtrip-tool"}],
        tools=[
            {
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "parameters": {"type": "object", "properties": {}},
                },
            }
        ],
    )
    tc = r.choices[0].message.tool_calls[0]
    assert tc.function.name == "get_weather", repr(tc)
    args = json.loads(tc.function.arguments)
    assert args["location"] == "San Francisco", repr(args)


check("non-streaming text", test_non_streaming_text)
check("streaming text", test_streaming_text)
check("tool call", test_tool_call)

print(f"\nResults: {PASS} passed, {FAIL} failed")
if FAIL:
    sys.exit(1)
