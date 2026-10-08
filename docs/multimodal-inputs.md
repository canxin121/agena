# Reading local media with the conversation model

`fs.read_media` supplies a local file's immutable bytes to the current
conversation model, through the same attachment contract used by user uploads.
It uses the selected provider, adapter, model and account; it does not invoke a
separate cloud-analysis model or require separate cloud-tool credentials.

Read the live schema through `tools_help`, then invoke it through `tools_call`:

```json
{"tool":"fs.read_media","input":{"path":"screenshots/error.png"}}
```

`path` resolves from the active workspace. `expected_sha256` optionally guards
the file revision. The result reports MIME type, byte size, SHA-256 and image
dimensions, and carries the actual media as a structured attachment. Base64 is
kept out of the text result and human presentation.

The session prompt advertises this workflow when the tool is available.
Metadata-only results from `fs.read` and local resource references also direct
the model to `fs.read_media`. Ordinary text should use `fs.read`; extracted
PDF/Office text should use `fs.document`.

## Protocol delivery

Model input support must be confirmed in the selected model's capabilities.
Protocol validation runs independently of capability overrides, before sending
the request. Declaring a model audio-capable cannot make Responses accept an
audio file as native audio input.

| Selected protocol | Images | PDF | Audio | Video | Tool-result transport |
| --- | --- | --- | --- | --- | --- |
| OpenAI Responses | PNG/JPEG/WebP/GIF | Yes | Unsupported | Unsupported | `function_call_output.output` text/image/file blocks |
| OpenAI Chat Completions | PNG/JPEG/WebP/GIF | Yes | WAV/MP3 with an audio-capable model | Unsupported | Text tool replies followed by ordinary user media parts, including `input_audio` |
| Claude Messages | PNG/JPEG/WebP/GIF | Yes | Unsupported | Unsupported | `tool_result.content` text/image/document blocks |
| Gemini generateContent | PNG/JPEG/WebP | Yes | Supported audio formats with an audio-capable model | Supported video formats with a video-capable model | Gemini 3 image/PDF/text uses `functionResponse.parts.inlineData`; older models and audio/video use ordinary `inlineData` parts |

The Chat compatibility path emits all parallel tool replies before its media
message. History replay and continuation match media to each occurrence of a
tool call, including reused provider call IDs. Gemini thought signatures remain
on their corresponding function calls.

Bedrock's native Claude path also carries image/document blocks inside tool
results. GitLab applies the validator for its selected backend. The current
Ollama adapter renders text messages only, so it rejects actual media input
instead of replacing it with a filename hint.

## Preparation and limits

Local preparation identifies supported media signatures, checks image decoding
and dimensions, rejects special files and final symlinks, and reads from one
open descriptor. Files must be nonempty and at most 20 MiB; UTF-8 text through
this path is limited to 1 MiB. Unsupported binary documents should be extracted
with `fs.document` or converted explicitly. HEIF/AVIF images require conversion
to PNG/JPEG.

Inline transport has additional conservative budgets across the request's
media, including Base64 expansion: 49,000,000 encoded bytes for OpenAI,
31,000,000 for Claude and 19,000,000 for Gemini. Claude images also have a
10,000,000 encoded-byte limit and 8000-pixel dimension limit. The local image
preparer additionally caps decoded images at 40 megapixels and 16384 pixels per
dimension. These budgets leave room for request framing; they do not guarantee
that arbitrary amounts of accompanying text fit an endpoint's full body limit.

This tool currently sends inline snapshots. It does not automatically upload
large files through a provider Files API, transcribe unsupported audio, extract
video frames or convert image formats. Unsupported model/protocol/format/size
combinations fail explicitly. An error is not evidence that the model saw or
heard the media.

Snapshots are bound to the original provider/model/endpoint/account route.
Changing the file later does not change persisted media. Changing the model or
connection cannot silently resend those snapshots to another destination;
continue on the original route or explicitly attach the media in a new
conversation.

Protocol references:

- [OpenAI function calling](https://developers.openai.com/api/docs/guides/function-calling)
- [OpenAI file inputs](https://developers.openai.com/api/docs/guides/file-inputs)
- [OpenAI audio Chat Completions](https://developers.openai.com/api/docs/guides/audio-chat-completions)
- [Claude tool results](https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls)
- [Gemini function calling](https://ai.google.dev/gemini-api/docs/generate-content/function-calling)
