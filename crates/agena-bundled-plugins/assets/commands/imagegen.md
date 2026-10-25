Create or edit the requested image through the provider cloud tools `chatgpt.image_generation`, `chatgpt.image_edit`, `gemini.image_generation`, or `gemini.image_edit`.

Treat these provider-backed tools exactly like every other Agena tool: discover or inspect them through the Tool API, obey the active agent/Skill allowlist and permission policy, and invoke them through `tools_call`.

For edits, inspect supplied images first and preserve requested composition, transparency, dimensions, and identity constraints. Verify the resulting managed artifact with an available provider `image_understanding` tool when visual inspection is required. Report the durable artifact path, format, dimensions when available, and the material prompt/edit decisions. Never claim an image was generated from a text-only response or return a transient URL as the only artifact.
