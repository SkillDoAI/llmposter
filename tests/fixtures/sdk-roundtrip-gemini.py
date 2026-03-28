"""Gemini SDK round-trip tests against llmposter.

Run with: uv run --with 'google-genai' python3 sdk-roundtrip-gemini.py
Expects llmposter running on http://127.0.0.1:19871 with sdk-roundtrip.yaml fixtures.
"""

import sys

import google.genai as genai

BASE = "http://127.0.0.1:19871"
client = genai.Client(api_key="dummy", http_options={"base_url": BASE})
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
    r = client.models.generate_content(
        model="gemini-pro",
        contents="sdk-roundtrip-hello",
    )
    text = r.candidates[0].content.parts[0].text
    assert text == "Hello from llmposter mock!", repr(text)


def test_streaming_text():
    chunks = []
    for chunk in client.models.generate_content_stream(
        model="gemini-pro",
        contents="sdk-roundtrip-hello",
    ):
        for part in chunk.candidates[0].content.parts:
            if part.text:
                chunks.append(part.text)
    assert "llmposter" in "".join(chunks), repr(chunks)


check("non-streaming text", test_non_streaming_text)
check("streaming text", test_streaming_text)

print(f"\nResults: {PASS} passed, {FAIL} failed")
if FAIL:
    sys.exit(1)
